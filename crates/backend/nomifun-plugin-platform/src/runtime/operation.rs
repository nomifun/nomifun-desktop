use std::collections::BTreeMap;

use nomifun_agent_contracts::{CanonicalErrorCode, DigestHex, MiniAppId, OperationId};
use nomifun_api_types::{
    DurableOperationKindDto, DurableOperationOwnerDto, DurableOperationStateDto,
    DurableOperationSummaryDto,
};
use serde::{Deserialize, Serialize};

use crate::runtime::{PluginRuntimePlatformError, PluginRuntimePlatformResult};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeOperationKind {
    Build,
    Import,
    Export,
    PermanentDelete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntimeOperationState {
    Running,
    Succeeded,
    Failed,
    Canceled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DurablePluginRuntimeOperation {
    pub operation_id: OperationId,
    pub revision: u64,
    pub miniapp_id: MiniAppId,
    pub kind: PluginRuntimeOperationKind,
    pub state: PluginRuntimeOperationState,
    pub cancelable: bool,
    pub progress_percent: Option<u8>,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub result_artifact_digests: BTreeMap<String, DigestHex>,
    pub last_error: Option<CanonicalErrorCode>,
}

impl DurablePluginRuntimeOperation {
    pub fn running(
        operation_id: OperationId,
        miniapp_id: MiniAppId,
        kind: PluginRuntimeOperationKind,
        cancelable: bool,
        now_ms: i64,
    ) -> PluginRuntimePlatformResult<Self> {
        let value = Self {
            operation_id,
            revision: 1,
            miniapp_id,
            kind,
            state: PluginRuntimeOperationState::Running,
            cancelable,
            progress_percent: if kind == PluginRuntimeOperationKind::PermanentDelete {
                None
            } else {
                Some(0)
            },
            started_at_ms: now_ms,
            completed_at_ms: None,
            result_artifact_digests: BTreeMap::new(),
            last_error: None,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn succeed(
        &mut self,
        expected_revision: u64,
        completed_at_ms: i64,
        artifacts: BTreeMap<String, DigestHex>,
    ) -> PluginRuntimePlatformResult<()> {
        self.require_running(expected_revision)?;
        self.revision += 1;
        self.state = PluginRuntimeOperationState::Succeeded;
        self.progress_percent = if self.kind == PluginRuntimeOperationKind::PermanentDelete {
            None
        } else {
            Some(100)
        };
        self.completed_at_ms = Some(completed_at_ms);
        self.result_artifact_digests = artifacts;
        self.last_error = None;
        self.validate()
    }

    pub fn fail(
        &mut self,
        expected_revision: u64,
        completed_at_ms: i64,
        error: CanonicalErrorCode,
    ) -> PluginRuntimePlatformResult<()> {
        self.require_running(expected_revision)?;
        self.revision += 1;
        self.state = PluginRuntimeOperationState::Failed;
        self.completed_at_ms = Some(completed_at_ms);
        self.last_error = Some(error);
        self.validate()
    }

    pub fn cancel(&mut self, expected_revision: u64, now_ms: i64) -> PluginRuntimePlatformResult<()> {
        self.require_running(expected_revision)?;
        if !self.cancelable {
            return Err(PluginRuntimePlatformError::OperationNotCancelable(
                self.operation_id.0.clone(),
            ));
        }
        self.revision += 1;
        self.state = PluginRuntimeOperationState::Canceled;
        self.completed_at_ms = Some(now_ms);
        self.validate()
    }

    fn require_running(&self, expected_revision: u64) -> PluginRuntimePlatformResult<()> {
        if self.revision != expected_revision || self.state != PluginRuntimeOperationState::Running {
            return Err(PluginRuntimePlatformError::OperationConflict);
        }
        Ok(())
    }

    pub fn validate(&self) -> PluginRuntimePlatformResult<()> {
        if self.operation_id.as_ref().trim().is_empty()
            || self.miniapp_id.as_ref().trim().is_empty()
            || self.revision == 0
            || self.started_at_ms <= 0
            || self.progress_percent.is_some_and(|value| value > 100)
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "durable operation identity or progress is invalid".into(),
            ));
        }
        if self.kind == PluginRuntimeOperationKind::PermanentDelete && self.progress_percent.is_some() {
            return Err(PluginRuntimePlatformError::InvalidState(
                "permanent delete must not persist percentage progress".into(),
            ));
        }
        match self.state {
            PluginRuntimeOperationState::Running => {
                if self.completed_at_ms.is_some() || self.last_error.is_some() {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "running operation cannot be completed or failed".into(),
                    ));
                }
            }
            PluginRuntimeOperationState::Succeeded => {
                if self.completed_at_ms.is_none()
                    || self.last_error.is_some()
                    || (self.kind != PluginRuntimeOperationKind::PermanentDelete
                        && self.progress_percent != Some(100))
                {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "succeeded operation has inconsistent terminal state".into(),
                    ));
                }
            }
            PluginRuntimeOperationState::Failed => {
                if self.completed_at_ms.is_none() || self.last_error.is_none() {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "failed operation requires completion and error".into(),
                    ));
                }
            }
            PluginRuntimeOperationState::Canceled => {
                if self.completed_at_ms.is_none() || self.last_error.is_some() {
                    return Err(PluginRuntimePlatformError::InvalidState(
                        "canceled operation has inconsistent terminal state".into(),
                    ));
                }
            }
        }
        if self
            .completed_at_ms
            .is_some_and(|completed| completed < self.started_at_ms)
        {
            return Err(PluginRuntimePlatformError::InvalidState(
                "operation completion precedes start".into(),
            ));
        }
        Ok(())
    }

    pub fn to_dto(&self) -> DurableOperationSummaryDto {
        DurableOperationSummaryDto {
            operation_id: self.operation_id.0.clone(),
            operation_revision: self.revision,
            kind: match self.kind {
                PluginRuntimeOperationKind::Build => DurableOperationKindDto::Build,
                PluginRuntimeOperationKind::Import => DurableOperationKindDto::Import,
                PluginRuntimeOperationKind::Export => DurableOperationKindDto::Export,
                PluginRuntimeOperationKind::PermanentDelete => {
                    DurableOperationKindDto::MiniappPermanentDelete
                }
            },
            owner: DurableOperationOwnerDto::Miniapp {
                miniapp_id: self.miniapp_id.0.clone(),
            },
            state: match self.state {
                PluginRuntimeOperationState::Running => DurableOperationStateDto::Running,
                PluginRuntimeOperationState::Succeeded => DurableOperationStateDto::Succeeded,
                PluginRuntimeOperationState::Failed => DurableOperationStateDto::Failed,
                PluginRuntimeOperationState::Canceled => DurableOperationStateDto::Canceled,
            },
            cancelable: self.cancelable,
            progress_percent: self.progress_percent,
            started_at_ms: self.started_at_ms,
            completed_at_ms: self.completed_at_ms,
        }
    }
}
