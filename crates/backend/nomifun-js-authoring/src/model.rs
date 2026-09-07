use std::sync::atomic::{AtomicBool, Ordering};

use nomifun_agent_contracts::{PluginProjectId, UserId};
use serde::{Deserialize, Serialize};
use uuid::{Uuid, Variant};

use crate::error::AuthoringError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceScope {
    owner_id: UserId,
    project_id: PluginProjectId,
}

impl SourceScope {
    pub fn new(
        owner_id: UserId,
        project_id: PluginProjectId,
    ) -> Result<Self, AuthoringError> {
        validate_uuid_v7(owner_id.as_ref(), "owner_id")?;
        validate_uuid_v7(project_id.as_ref(), "project_id")?;
        Ok(Self {
            owner_id,
            project_id,
        })
    }

    pub fn owner_id(&self) -> &UserId {
        &self.owner_id
    }

    pub fn project_id(&self) -> &PluginProjectId {
        &self.project_id
    }
}

fn validate_uuid_v7(
    value: &str,
    field: &'static str,
) -> Result<(), AuthoringError> {
    let parsed =
        Uuid::parse_str(value).map_err(|error| AuthoringError::InvalidField {
            field,
            reason: format!("must be a canonical UUIDv7: {error}"),
        })?;
    if parsed.get_version_num() != 7
        || parsed.get_variant() != Variant::RFC4122
        || parsed.to_string() != value
    {
        return Err(AuthoringError::InvalidField {
            field,
            reason: "must be a lowercase canonical RFC 4122 UUIDv7".into(),
        });
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SourceStoreLimits {
    pub max_file_count: usize,
    pub max_package_json_bytes: u64,
    pub max_single_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for SourceStoreLimits {
    fn default() -> Self {
        Self {
            max_file_count: 4_096,
            max_package_json_bytes: 1024 * 1024,
            max_single_file_bytes: 64 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
        }
    }
}

impl SourceStoreLimits {
    pub(crate) fn validate(self) -> Result<Self, AuthoringError> {
        if self.max_file_count == 0
            || self.max_package_json_bytes == 0
            || self.max_single_file_bytes == 0
            || self.max_package_json_bytes > self.max_single_file_bytes
            || self.max_total_bytes < self.max_single_file_bytes
        {
            Err(AuthoringError::InvalidLimits)
        } else {
            Ok(self)
        }
    }
}

pub trait OperationCancellation: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

#[derive(Debug, Default)]
pub struct NeverCancel;

impl OperationCancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

#[derive(Debug, Default)]
pub struct CancellationFlag {
    canceled: AtomicBool,
}

impl CancellationFlag {
    pub fn cancel(&self) {
        self.canceled.store(true, Ordering::Release);
    }
}

impl OperationCancellation for CancellationFlag {
    fn is_cancelled(&self) -> bool {
        self.canceled.load(Ordering::Acquire)
    }
}

pub(crate) fn check_canceled(
    cancellation: &dyn OperationCancellation,
) -> Result<(), AuthoringError> {
    if cancellation.is_cancelled() {
        Err(AuthoringError::Canceled)
    } else {
        Ok(())
    }
}
