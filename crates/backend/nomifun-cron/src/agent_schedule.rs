//! Product-facing `automation.schedule` actions.
//!
//! The timer, wake-up loop, retry policy, and scheduled-turn trigger remain
//! private [`CronService`] concerns.  Agent authoring receives only the four
//! schedule-management actions declared here.  Every call is additionally
//! narrowed by one owner-scoped `scheduler` resource selected in the resolved
//! AgentSession snapshot; an action grant alone is never enough.

use std::collections::BTreeSet;
use std::sync::Arc;

use nomifun_api_types::{CronJobResponse, ListCronJobsQuery};
use nomifun_common::{ConversationId, CronJobId, UserId};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::service::{
    CronEmbeddedCreateCommand, CronEmbeddedDeleteCommand, CronEmbeddedMutationRequest,
    CronEmbeddedUpdateCommand, CronService,
};
use crate::types::cron_job_to_response;

pub const AUTOMATION_SCHEDULE_MODULE_ID: &str = "automation.schedule";
pub const SCHEDULE_LIST_ACTION_ID: &str = "automation.schedule/list";
pub const SCHEDULE_CREATE_ACTION_ID: &str = "automation.schedule/create";
pub const SCHEDULE_UPDATE_ACTION_ID: &str = "automation.schedule/update";
pub const SCHEDULE_DELETE_ACTION_ID: &str = "automation.schedule/delete";
pub const AUTOMATION_SCHEDULE_ACTION_IDS: [&str; 4] = [
    SCHEDULE_LIST_ACTION_ID,
    SCHEDULE_CREATE_ACTION_ID,
    SCHEDULE_UPDATE_ACTION_ID,
    SCHEDULE_DELETE_ACTION_ID,
];

pub const SCHEDULER_RESOURCE_KIND: &str = "scheduler";
pub const SCHEDULER_READ_OPERATION: &str = "read";
pub const SCHEDULER_WRITE_OPERATION: &str = "write";
pub const SCHEDULER_DELETE_OPERATION: &str = "delete";
pub const SCHEDULER_RESOURCE_OPERATIONS: [&str; 3] = [
    SCHEDULER_READ_OPERATION,
    SCHEDULER_WRITE_OPERATION,
    SCHEDULER_DELETE_OPERATION,
];

const MAX_NAME_CHARS: usize = 256;
const MAX_SCHEDULE_CHARS: usize = 512;
const MAX_DESCRIPTION_CHARS: usize = 2_048;
const MAX_MESSAGE_CHARS: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScheduleAction {
    List,
    Create,
    Update,
    Delete,
}

impl ScheduleAction {
    pub const fn action_id(self) -> &'static str {
        match self {
            Self::List => SCHEDULE_LIST_ACTION_ID,
            Self::Create => SCHEDULE_CREATE_ACTION_ID,
            Self::Update => SCHEDULE_UPDATE_ACTION_ID,
            Self::Delete => SCHEDULE_DELETE_ACTION_ID,
        }
    }

    pub const fn required_resource_operation(self) -> ScheduleResourceOperation {
        match self {
            Self::List => ScheduleResourceOperation::Read,
            Self::Create | Self::Update => ScheduleResourceOperation::Write,
            Self::Delete => ScheduleResourceOperation::Delete,
        }
    }

    pub fn from_action_id(action_id: &str) -> Option<Self> {
        match action_id {
            SCHEDULE_LIST_ACTION_ID => Some(Self::List),
            SCHEDULE_CREATE_ACTION_ID => Some(Self::Create),
            SCHEDULE_UPDATE_ACTION_ID => Some(Self::Update),
            SCHEDULE_DELETE_ACTION_ID => Some(Self::Delete),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ScheduleResourceOperation {
    Read,
    Write,
    Delete,
}

impl ScheduleResourceOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => SCHEDULER_READ_OPERATION,
            Self::Write => SCHEDULER_WRITE_OPERATION,
            Self::Delete => SCHEDULER_DELETE_OPERATION,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            SCHEDULER_READ_OPERATION => Some(Self::Read),
            SCHEDULER_WRITE_OPERATION => Some(Self::Write),
            SCHEDULER_DELETE_OPERATION => Some(Self::Delete),
            _ => None,
        }
    }
}

/// One concrete scheduler owner selected for a resolved AgentSession.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleResourceBinding {
    binding_id: String,
    scheduler_id: String,
    owner_id: String,
    operations: BTreeSet<ScheduleResourceOperation>,
}

impl ScheduleResourceBinding {
    pub fn from_operation_names<I, S>(
        binding_id: impl Into<String>,
        scheduler_id: impl Into<String>,
        owner_id: impl Into<String>,
        operations: I,
    ) -> Result<Self, ScheduleActionError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let operations = operations
            .into_iter()
            .map(|operation| {
                ScheduleResourceOperation::parse(operation.as_ref()).ok_or_else(|| {
                    ScheduleActionError::InvalidResource(format!(
                        "scheduler resource contains unsupported operation {}",
                        operation.as_ref()
                    ))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(binding_id, scheduler_id, owner_id, operations)
    }

    pub fn new(
        binding_id: impl Into<String>,
        scheduler_id: impl Into<String>,
        owner_id: impl Into<String>,
        operations: impl IntoIterator<Item = ScheduleResourceOperation>,
    ) -> Result<Self, ScheduleActionError> {
        let binding_id = binding_id.into();
        let scheduler_id = scheduler_id.into();
        let owner_id = owner_id.into();
        if binding_id.trim().is_empty()
            || scheduler_id.trim().is_empty()
            || owner_id.trim().is_empty()
        {
            return Err(ScheduleActionError::InvalidResource(
                "scheduler binding, resource, and owner identities must not be blank".into(),
            ));
        }
        UserId::try_from(owner_id.as_str()).map_err(|error| {
            ScheduleActionError::InvalidResource(format!(
                "scheduler resource owner is invalid: {error}"
            ))
        })?;
        let operations = operations.into_iter().collect::<BTreeSet<_>>();
        if operations.is_empty() {
            return Err(ScheduleActionError::InvalidResource(
                "scheduler resource must grant at least one operation".into(),
            ));
        }
        Ok(Self {
            binding_id,
            scheduler_id,
            owner_id,
            operations,
        })
    }

    pub fn binding_id(&self) -> &str {
        &self.binding_id
    }

    pub fn scheduler_id(&self) -> &str {
        &self.scheduler_id
    }

    pub fn operation_names(&self) -> BTreeSet<&'static str> {
        self.operations
            .iter()
            .map(|operation| operation.as_str())
            .collect()
    }

    fn authorize(
        &self,
        principal_id: &str,
        action: ScheduleAction,
    ) -> Result<(), ScheduleActionError> {
        if self.owner_id != principal_id {
            return Err(ScheduleActionError::ResourceOwnerMismatch);
        }
        let required = action.required_resource_operation();
        if !self.operations.contains(&required) {
            return Err(ScheduleActionError::ResourceOperationDenied {
                action_id: action.action_id(),
                operation: required.as_str(),
            });
        }
        Ok(())
    }
}

/// Frozen Schedule authority. `None` deliberately represents the user-visible
/// unbound state instead of silently manufacturing process-global authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleAuthority {
    principal_id: String,
    resource: Option<ScheduleResourceBinding>,
}

impl ScheduleAuthority {
    pub fn unbound(principal_id: impl Into<String>) -> Result<Self, ScheduleActionError> {
        Self::new(principal_id, None)
    }

    pub fn bound(
        principal_id: impl Into<String>,
        resource: ScheduleResourceBinding,
    ) -> Result<Self, ScheduleActionError> {
        Self::new(principal_id, Some(resource))
    }

    fn new(
        principal_id: impl Into<String>,
        resource: Option<ScheduleResourceBinding>,
    ) -> Result<Self, ScheduleActionError> {
        let principal_id = principal_id.into();
        UserId::try_from(principal_id.as_str()).map_err(|error| {
            ScheduleActionError::InvalidContext(format!(
                "schedule principal identity is invalid: {error}"
            ))
        })?;
        if resource
            .as_ref()
            .is_some_and(|resource| resource.owner_id != principal_id)
        {
            return Err(ScheduleActionError::ResourceOwnerMismatch);
        }
        Ok(Self {
            principal_id,
            resource,
        })
    }

    pub fn principal_id(&self) -> &str {
        &self.principal_id
    }

    pub fn selection(&self) -> ScheduleResourceSelection {
        match self.resource.as_ref() {
            Some(resource) => ScheduleResourceSelection::Bound {
                binding_id: resource.binding_id.clone(),
                scheduler_id: resource.scheduler_id.clone(),
                operations: resource.operation_names().into_iter().map(str::to_owned).collect(),
            },
            None => ScheduleResourceSelection::Unbound,
        }
    }

    fn require(
        &self,
        action: ScheduleAction,
    ) -> Result<&ScheduleResourceBinding, ScheduleActionError> {
        let resource = self
            .resource
            .as_ref()
            .ok_or(ScheduleActionError::SchedulerUnbound)?;
        resource.authorize(&self.principal_id, action)?;
        Ok(resource)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ScheduleResourceSelection {
    Unbound,
    Bound {
        binding_id: String,
        scheduler_id: String,
        operations: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleActionContext {
    pub principal_id: String,
    pub agent_session_id: String,
    pub operation_id: String,
}

impl ScheduleActionContext {
    fn validate(&self, authority: &ScheduleAuthority) -> Result<(), ScheduleActionError> {
        if self.principal_id != authority.principal_id {
            return Err(ScheduleActionError::ResourceOwnerMismatch);
        }
        UserId::try_from(self.principal_id.as_str()).map_err(|error| {
            ScheduleActionError::InvalidContext(format!("invalid principal identity: {error}"))
        })?;
        ConversationId::try_from(self.agent_session_id.as_str()).map_err(|error| {
            ScheduleActionError::InvalidContext(format!("invalid AgentSession identity: {error}"))
        })?;
        if self.operation_id.is_empty()
            || self.operation_id.len() > 256
            || !self
                .operation_id
                .bytes()
                .all(|byte| (0x21..=0x7e).contains(&byte))
        {
            return Err(ScheduleActionError::InvalidContext(
                "operation identity must contain 1 to 256 visible ASCII bytes".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScheduleListInput {}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCreateInput {
    pub name: String,
    pub schedule: String,
    #[serde(default)]
    pub schedule_description: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScheduleUpdateInput {
    pub cron_job_id: CronJobId,
    pub name: String,
    pub schedule: String,
    #[serde(default)]
    pub schedule_description: Option<String>,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDeleteInput {
    pub cron_job_id: CronJobId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleExternalActionStatus {
    Succeeded,
    AlreadyExists,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct ScheduleListOutput {
    pub status: ScheduleExternalActionStatus,
    pub jobs: Vec<CronJobResponse>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ScheduleMutationOutput {
    pub status: ScheduleExternalActionStatus,
    pub cron_job_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Error)]
pub enum ScheduleActionError {
    #[error("automation.schedule has no bound scheduler resource")]
    SchedulerUnbound,
    #[error("scheduler resource belongs to a different principal")]
    ResourceOwnerMismatch,
    #[error("{action_id} requires scheduler operation {operation}")]
    ResourceOperationDenied {
        action_id: &'static str,
        operation: &'static str,
    },
    #[error("invalid scheduler resource: {0}")]
    InvalidResource(String),
    #[error("invalid schedule action context: {0}")]
    InvalidContext(String),
    #[error("invalid schedule action input: {0}")]
    InvalidInput(String),
    #[error("schedule job is not bound to the invoking AgentSession")]
    JobOutsideSession,
    #[error("schedule owner failed: {0}")]
    Service(String),
    #[error("schedule mutation outcome is unknown: {0}")]
    OutcomeUnknown(String),
}

/// Domain owner used by the canonical Capability host.
///
/// The caller cannot supply a provider, model, target Session, timer handle, or
/// trigger implementation.  All of those stay behind [`CronService`].
#[derive(Clone)]
pub struct ScheduleActionOwner {
    service: Arc<CronService>,
}

impl ScheduleActionOwner {
    pub fn new(service: Arc<CronService>) -> Self {
        Self { service }
    }

    pub async fn list(
        &self,
        authority: &ScheduleAuthority,
        context: &ScheduleActionContext,
        _input: ScheduleListInput,
    ) -> Result<ScheduleListOutput, ScheduleActionError> {
        context.validate(authority)?;
        authority.require(ScheduleAction::List)?;
        let jobs = self
            .service
            .list_jobs(
                authority.principal_id(),
                &ListCronJobsQuery {
                    conversation_id: Some(context.agent_session_id.clone()),
                },
            )
            .await
            .map_err(service_error)?;
        Ok(ScheduleListOutput {
            status: ScheduleExternalActionStatus::Succeeded,
            jobs: jobs.iter().map(cron_job_to_response).collect(),
        })
    }

    pub async fn create(
        &self,
        authority: &ScheduleAuthority,
        context: &ScheduleActionContext,
        input: ScheduleCreateInput,
    ) -> Result<ScheduleMutationOutput, ScheduleActionError> {
        context.validate(authority)?;
        authority.require(ScheduleAction::Create)?;
        validate_create_fields(
            &input.name,
            &input.schedule,
            input.schedule_description.as_deref(),
            &input.message,
        )?;

        let existing = self
            .service
            .list_jobs(
                authority.principal_id(),
                &ListCronJobsQuery {
                    conversation_id: Some(context.agent_session_id.clone()),
                },
            )
            .await
            .map_err(service_error)?
            .into_iter()
            .find(|job| {
                job.enabled
                    && (job.name.trim().eq_ignore_ascii_case(input.name.trim())
                        || job.message.trim() == input.message.trim())
            });
        if let Some(existing) = existing {
            return Ok(ScheduleMutationOutput {
                status: ScheduleExternalActionStatus::AlreadyExists,
                cron_job_id: Some(existing.cron_job_id),
                message: "an active schedule with the same name or message already exists"
                    .into(),
            });
        }

        let created_name = input.name.clone();
        let created_message = input.message.clone();
        let waiter = self
            .service
            .submit_embedded_create(
                authority.principal_id(),
                &context.agent_session_id,
                CronEmbeddedMutationRequest {
                    operation_id: canonical_operation_id(context),
                    command: CronEmbeddedCreateCommand {
                        name: input.name,
                        schedule: input.schedule,
                        schedule_description: input.schedule_description.unwrap_or_default(),
                        message: input.message,
                    },
                },
            )
            .map_err(service_error)?;
        let result = waiter.wait().await.map_err(schedule_wait_error)?;
        let cron_job_id = if result.success {
            self.service
                .list_jobs(
                    authority.principal_id(),
                    &ListCronJobsQuery {
                        conversation_id: Some(context.agent_session_id.clone()),
                    },
                )
                .await
                .ok()
                .and_then(|jobs| {
                    jobs.into_iter()
                        .filter(|job| job.name == created_name && job.message == created_message)
                        .max_by_key(|job| job.created_at)
                        .map(|job| job.cron_job_id)
                })
        } else {
            None
        };
        Ok(command_output(
            result.success,
            cron_job_id,
            result.message,
        ))
    }

    pub async fn update(
        &self,
        authority: &ScheduleAuthority,
        context: &ScheduleActionContext,
        input: ScheduleUpdateInput,
    ) -> Result<ScheduleMutationOutput, ScheduleActionError> {
        context.validate(authority)?;
        authority.require(ScheduleAction::Update)?;
        validate_create_fields(
            &input.name,
            &input.schedule,
            input.schedule_description.as_deref(),
            &input.message,
        )?;
        let cron_job_id = input.cron_job_id.into_string();
        let waiter = self
            .service
            .submit_embedded_update(
                authority.principal_id(),
                &context.agent_session_id,
                CronEmbeddedMutationRequest {
                    operation_id: canonical_operation_id(context),
                    command: CronEmbeddedUpdateCommand {
                        job_id: cron_job_id.clone(),
                        name: input.name,
                        schedule: input.schedule,
                        schedule_description: input.schedule_description.unwrap_or_default(),
                        message: input.message,
                    },
                },
            )
            .map_err(service_error)?;
        let result = waiter.wait().await.map_err(schedule_wait_error)?;
        Ok(command_output(
            result.success,
            Some(cron_job_id),
            result.message,
        ))
    }

    pub async fn delete(
        &self,
        authority: &ScheduleAuthority,
        context: &ScheduleActionContext,
        input: ScheduleDeleteInput,
    ) -> Result<ScheduleMutationOutput, ScheduleActionError> {
        context.validate(authority)?;
        authority.require(ScheduleAction::Delete)?;
        let cron_job_id = input.cron_job_id.into_string();
        // Deletion in Cron is owner-scoped.  The product action is narrower:
        // it may delete only a job bound to the invoking AgentSession.
        let job = self
            .service
            .get_job(authority.principal_id(), &cron_job_id)
            .await
            .map_err(service_error)?;
        if job.conversation_id.as_deref() != Some(context.agent_session_id.as_str()) {
            return Err(ScheduleActionError::JobOutsideSession);
        }
        let waiter = self
            .service
            .submit_embedded_delete(
                authority.principal_id(),
                CronEmbeddedMutationRequest {
                    operation_id: canonical_operation_id(context),
                    command: CronEmbeddedDeleteCommand {
                        job_id: cron_job_id.clone(),
                    },
                },
            )
            .map_err(service_error)?;
        let result = waiter.wait().await.map_err(schedule_wait_error)?;
        Ok(command_output(
            result.success,
            Some(cron_job_id),
            result.message,
        ))
    }
}

fn canonical_operation_id(context: &ScheduleActionContext) -> String {
    context.operation_id.clone()
}

fn command_output(
    success: bool,
    cron_job_id: Option<String>,
    message: String,
) -> ScheduleMutationOutput {
    ScheduleMutationOutput {
        status: if success {
            ScheduleExternalActionStatus::Succeeded
        } else {
            ScheduleExternalActionStatus::Failed
        },
        cron_job_id,
        message,
    }
}

fn service_error(error: impl std::fmt::Display) -> ScheduleActionError {
    ScheduleActionError::Service(error.to_string())
}

fn schedule_wait_error(error: crate::error::CronError) -> ScheduleActionError {
    match error {
        crate::error::CronError::OutcomeUnknown(message) => {
            ScheduleActionError::OutcomeUnknown(message)
        }
        other => ScheduleActionError::Service(other.to_string()),
    }
}

fn validate_create_fields(
    name: &str,
    schedule: &str,
    schedule_description: Option<&str>,
    message: &str,
) -> Result<(), ScheduleActionError> {
    validate_text("name", name, MAX_NAME_CHARS)?;
    validate_text("schedule", schedule, MAX_SCHEDULE_CHARS)?;
    if let Some(description) = schedule_description {
        validate_optional_text("schedule_description", description, MAX_DESCRIPTION_CHARS)?;
    }
    validate_text("message", message, MAX_MESSAGE_CHARS)
}

fn validate_text(label: &str, value: &str, max: usize) -> Result<(), ScheduleActionError> {
    if value.trim().is_empty() || value.chars().count() > max {
        return Err(ScheduleActionError::InvalidInput(format!(
            "{label} must contain 1 to {max} characters"
        )));
    }
    Ok(())
}

fn validate_optional_text(
    label: &str,
    value: &str,
    max: usize,
) -> Result<(), ScheduleActionError> {
    if value.chars().count() > max {
        return Err(ScheduleActionError::InvalidInput(format!(
            "{label} must contain at most {max} characters"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const OWNER: &str = "0190f5fe-7c00-7a00-8000-000000000001";

    fn binding(
        owner: &str,
        operations: impl IntoIterator<Item = ScheduleResourceOperation>,
    ) -> ScheduleResourceBinding {
        ScheduleResourceBinding::new("binding-a", "local-scheduler", owner, operations).unwrap()
    }

    #[test]
    fn authoring_surface_contains_only_four_product_actions() {
        assert_eq!(
            AUTOMATION_SCHEDULE_ACTION_IDS,
            [
                "automation.schedule/list",
                "automation.schedule/create",
                "automation.schedule/update",
                "automation.schedule/delete",
            ]
        );
        assert!(ScheduleAction::from_action_id("automation.schedule/timer").is_none());
        assert!(ScheduleAction::from_action_id("automation.schedule/trigger").is_none());
    }

    #[test]
    fn bound_and_unbound_resource_states_are_explicit() {
        let unbound = ScheduleAuthority::unbound(OWNER).unwrap();
        assert_eq!(unbound.selection(), ScheduleResourceSelection::Unbound);
        assert!(matches!(
            unbound.require(ScheduleAction::List),
            Err(ScheduleActionError::SchedulerUnbound)
        ));

        let bound = ScheduleAuthority::bound(
            OWNER,
            binding(OWNER, [ScheduleResourceOperation::Read]),
        )
        .unwrap();
        assert!(matches!(
            bound.selection(),
            ScheduleResourceSelection::Bound { .. }
        ));
        assert!(bound.require(ScheduleAction::List).is_ok());
        assert!(matches!(
            bound.require(ScheduleAction::Create),
            Err(ScheduleActionError::ResourceOperationDenied { .. })
        ));
    }

    #[test]
    fn resource_authority_rejects_owner_mismatch_and_unknown_operations() {
        let other = "0190f5fe-7c00-7a00-8000-000000000002";
        assert!(ScheduleAuthority::bound(OWNER, binding(other, [ScheduleResourceOperation::Read])).is_err());
        assert!(ScheduleResourceBinding::from_operation_names(
            "binding-a",
            "local-scheduler",
            OWNER,
            ["read", "timer"],
        )
        .is_err());
    }

    #[test]
    fn action_inputs_reject_provider_session_and_scheduler_mechanics() {
        assert!(serde_json::from_value::<ScheduleCreateInput>(json!({
            "name": "Daily",
            "schedule": "0 9 * * *",
            "message": "Run",
            "provider_id": "forbidden"
        })).is_err());
        assert!(serde_json::from_value::<ScheduleCreateInput>(json!({
            "name": "Daily",
            "schedule": "0 9 * * *",
            "message": "Run",
            "conversation_id": "forbidden"
        })).is_err());
        assert!(serde_json::from_value::<ScheduleCreateInput>(json!({
            "name": "Daily",
            "schedule": "0 9 * * *",
            "message": "Run",
            "timer_id": "forbidden"
        })).is_err());
    }

    #[test]
    fn external_action_status_is_machine_readable() {
        assert_eq!(
            serde_json::to_value(command_output(true, Some("job-a".into()), "ok".into()))
                .unwrap()["status"],
            "succeeded"
        );
        assert_eq!(
            serde_json::to_value(command_output(false, None, "failed".into())).unwrap()["status"],
            "failed"
        );
    }
}
