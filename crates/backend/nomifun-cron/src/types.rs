use std::str::FromStr;

use nomifun_api_types::{
    AgentBindingValueDto, AgentChatModelSelectionDto, CronJobMetadataDto, CronJobResponse,
    CronJobStateDto, CronScheduleDto, ProductAgentSelectionDto,
};
use nomifun_common::{ConversationId, CronJobId, TimestampMs, UserId};
use nomifun_db::models::CronJobRow;
use serde::{Deserialize, Serialize};

use crate::error::CronError;

// ---------------------------------------------------------------------------
// Domain enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum CronSchedule {
    At {
        at_ms: TimestampMs,
        description: Option<String>,
    },
    Every {
        every_ms: i64,
        description: Option<String>,
    },
    Cron {
        expr: String,
        tz: Option<String>,
        description: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Existing,
    NewConversation,
}

impl ExecutionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Existing => "existing",
            Self::NewConversation => "new_conversation",
        }
    }
}

impl FromStr for ExecutionMode {
    type Err = CronError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "existing" => Ok(Self::Existing),
            "new_conversation" => Ok(Self::NewConversation),
            other => Err(CronError::InvalidExecutionMode(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CreatedBy {
    User,
    Agent,
}

impl CreatedBy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }
}

impl FromStr for CreatedBy {
    type Err = CronError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(Self::User),
            "agent" => Ok(Self::Agent),
            other => Err(CronError::InvalidCreatedBy(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatus {
    Ok,
    Error,
    Skipped,
    Missed,
    #[serde(rename = "input_required")]
    InputRequired,
    Cancelled,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Skipped => "skipped",
            Self::Missed => "missed",
            Self::InputRequired => "input_required",
            Self::Cancelled => "cancelled",
        }
    }
}

impl FromStr for JobStatus {
    type Err = CronError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ok" => Ok(Self::Ok),
            "error" => Ok(Self::Error),
            "skipped" => Ok(Self::Skipped),
            "missed" => Ok(Self::Missed),
            "input_required" => Ok(Self::InputRequired),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(CronError::InvalidJobStatus(other.to_owned())),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CronJob {
    pub cron_job_id: String,
    pub user_id: String,
    pub name: String,
    pub enabled: bool,
    pub schedule_revision: i64,
    pub schedule: CronSchedule,
    pub message: String,
    pub execution_mode: ExecutionMode,
    /// Authoring intent is never consulted to execute an occurrence.
    pub agent_selection: Option<ProductAgentSelectionDto>,
    pub model: Option<AgentChatModelSelectionDto>,
    pub agent_binding: Option<AgentBindingValueDto>,
    pub agent_label: Option<String>,
    /// Exact immutable model presentation; never serialized into job storage.
    pub effective_model: Option<AgentChatModelSelectionDto>,
    pub clear_context_each_run: bool,
    pub configuration_error: Option<String>,
    pub last_failure: Option<nomifun_api_types::AgentStreamErrorData>,
    pub conversation_id: Option<String>,
    pub conversation_title: Option<String>,
    pub created_by: CreatedBy,
    pub skill_content: Option<String>,
    pub description: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    pub next_run_at: Option<TimestampMs>,
    pub last_run_at: Option<TimestampMs>,
    pub last_run_id: Option<String>,
    pub last_status: Option<JobStatus>,
    pub last_error: Option<String>,
    pub run_count: i64,
}

pub fn cron_job_from_row(row: CronJobRow) -> Result<CronJob, CronError> {
    CronJobId::parse(&row.cron_job_id).map_err(|error| CronError::Scheduler(error.to_string()))?;
    UserId::parse(&row.user_id).map_err(|error| CronError::Scheduler(error.to_string()))?;
    if let Some(session) = row.conversation_id.as_deref() {
        ConversationId::parse(session).map_err(|error| CronError::Scheduler(error.to_string()))?;
    }
    Ok(CronJob {
        cron_job_id: row.cron_job_id,
        user_id: row.user_id,
        name: row.name,
        enabled: row.enabled,
        schedule_revision: row.schedule_revision,
        schedule: parse_schedule(
            &row.schedule_kind,
            &row.schedule_value,
            row.schedule_tz.as_deref(),
            row.schedule_description.as_deref(),
        )?,
        message: row.payload_message,
        execution_mode: ExecutionMode::from_str(&row.execution_mode)?,
        agent_selection: row
            .agent_selection
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?,
        model: row
            .model_selection
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?,
        agent_binding: row
            .agent_binding
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?,
        agent_label: None,
        effective_model: None,
        clear_context_each_run: row.clear_context_each_run,
        configuration_error: row.configuration_error,
        last_failure: None,
        conversation_id: row.conversation_id,
        conversation_title: row.conversation_title,
        created_by: CreatedBy::from_str(&row.created_by)?,
        skill_content: row.skill_content,
        description: row.description,
        created_at: row.created_at,
        updated_at: row.updated_at,
        next_run_at: row.next_run_at,
        last_run_at: row.last_run_at,
        last_run_id: row.last_run_id,
        last_status: row
            .last_status
            .as_deref()
            .map(JobStatus::from_str)
            .transpose()?,
        last_error: row.last_error,
        run_count: row.run_count,
    })
}

pub fn cron_job_to_row(job: &CronJob) -> Result<CronJobRow, CronError> {
    CronJobId::parse(&job.cron_job_id).map_err(|error| CronError::Scheduler(error.to_string()))?;
    UserId::parse(&job.user_id).map_err(|error| CronError::Scheduler(error.to_string()))?;
    let (schedule_kind, schedule_value, schedule_tz, schedule_description) =
        schedule_to_row_fields(&job.schedule);
    Ok(CronJobRow {
        id: 0,
        cron_job_id: job.cron_job_id.clone(),
        user_id: job.user_id.clone(),
        name: job.name.clone(),
        enabled: job.enabled,
        schedule_revision: job.schedule_revision,
        schedule_kind,
        schedule_value,
        schedule_tz,
        schedule_description,
        payload_message: job.message.clone(),
        execution_mode: job.execution_mode.as_str().into(),
        agent_selection: if job.execution_mode == ExecutionMode::Existing
            && job.conversation_id.is_some()
        {
            None
        } else {
            job.agent_selection
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?
        },
        model_selection: if job.execution_mode == ExecutionMode::Existing
            && job.conversation_id.is_some()
        {
            None
        } else {
            job.model.as_ref().map(serde_json::to_string).transpose()?
        },
        agent_binding: if job.execution_mode == ExecutionMode::Existing
            && job.conversation_id.is_some()
        {
            None
        } else {
            job.agent_binding
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?
        },
        clear_context_each_run: job.clear_context_each_run,
        configuration_error: job.configuration_error.clone(),
        conversation_id: job.conversation_id.clone(),
        conversation_title: job.conversation_title.clone(),
        created_by: job.created_by.as_str().into(),
        skill_content: job.skill_content.clone(),
        description: job.description.clone(),
        created_at: job.created_at,
        updated_at: job.updated_at,
        next_run_at: job.next_run_at,
        last_run_at: job.last_run_at,
        last_run_id: job.last_run_id.clone(),
        last_status: job.last_status.map(|status| status.as_str().into()),
        last_error: job.last_error.clone(),
        run_count: job.run_count,
    })
}

pub fn cron_job_to_response(job: &CronJob) -> CronJobResponse {
    let schedule = match &job.schedule {
        CronSchedule::At { at_ms, description } => CronScheduleDto::At {
            at_ms: *at_ms,
            description: description.clone(),
        },
        CronSchedule::Every {
            every_ms,
            description,
        } => CronScheduleDto::Every {
            every_ms: *every_ms,
            description: description.clone(),
        },
        CronSchedule::Cron {
            expr,
            tz,
            description,
        } => CronScheduleDto::Cron {
            expr: expr.clone(),
            tz: tz.clone(),
            description: description.clone(),
        },
    };
    CronJobResponse {
        cron_job_id: job.cron_job_id.clone(),
        name: job.name.clone(),
        description: job.description.clone(),
        enabled: job.enabled,
        schedule,
        message: job.message.clone(),
        execution_mode: job.execution_mode.as_str().into(),
        metadata: CronJobMetadataDto {
            conversation_id: job.conversation_id.clone(),
            conversation_title: job.conversation_title.clone(),
            created_by: job.created_by.as_str().into(),
            created_at: job.created_at,
            updated_at: job.updated_at,
            agent_selection: job.agent_selection.clone(),
            model: job.effective_model.clone().or(job.model.clone()),
            agent_binding: job.agent_binding.clone(),
            agent_label: job.agent_label.clone(),
            clear_context_each_run: job.clear_context_each_run,
        },
        state: CronJobStateDto {
            next_run_at_ms: job.next_run_at,
            last_run_at_ms: job.last_run_at,
            last_status: job.last_status.map(|status| status.as_str().into()),
            last_error: job.last_error.clone(),
            run_count: job.run_count,
            configuration_error: job.configuration_error.clone(),
            last_failure: job.last_failure.clone(),
        },
    }
}

fn parse_schedule(
    kind: &str,
    value: &str,
    tz: Option<&str>,
    description: Option<&str>,
) -> Result<CronSchedule, CronError> {
    match kind {
        "at" => {
            let at_ms = value
                .parse::<TimestampMs>()
                .map_err(|e| CronError::InvalidSchedule(format!("invalid at_ms '{value}': {e}")))?;
            Ok(CronSchedule::At {
                at_ms,
                description: description.map(String::from),
            })
        }
        "every" => {
            let every_ms = value.parse::<i64>().map_err(|e| {
                CronError::InvalidSchedule(format!("invalid every_ms '{value}': {e}"))
            })?;
            Ok(CronSchedule::Every {
                every_ms,
                description: description.map(String::from),
            })
        }
        "cron" => Ok(CronSchedule::Cron {
            expr: value.to_owned(),
            tz: tz.map(String::from),
            description: description.map(String::from),
        }),
        other => Err(CronError::InvalidSchedule(format!(
            "unknown schedule kind: {other}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Domain ↔ DB conversion
// ---------------------------------------------------------------------------

pub(crate) fn schedule_to_row_fields(
    schedule: &CronSchedule,
) -> (String, String, Option<String>, Option<String>) {
    match schedule {
        CronSchedule::At { at_ms, description } => (
            "at".to_owned(),
            at_ms.to_string(),
            None,
            description.clone(),
        ),
        CronSchedule::Every {
            every_ms,
            description,
        } => (
            "every".to_owned(),
            every_ms.to_string(),
            None,
            description.clone(),
        ),
        CronSchedule::Cron {
            expr,
            tz,
            description,
        } => (
            "cron".to_owned(),
            expr.clone(),
            tz.clone(),
            description.clone(),
        ),
    }
}

pub fn schedule_from_dto(dto: &CronScheduleDto) -> CronSchedule {
    match dto {
        CronScheduleDto::At { at_ms, description } => CronSchedule::At {
            at_ms: *at_ms,
            description: description.clone(),
        },
        CronScheduleDto::Every {
            every_ms,
            description,
        } => CronSchedule::Every {
            every_ms: *every_ms,
            description: description.clone(),
        },
        CronScheduleDto::Cron {
            expr,
            tz,
            description,
        } => CronSchedule::Cron {
            expr: expr.clone(),
            tz: tz.clone(),
            description: description.clone(),
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    const JOB_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678901";
    const USER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000001";
    const PROVIDER_ID: &str = "0190f5fe-7c00-7a00-8000-000000000002";

    // -- Enum parsing ---------------------------------------------------------

    #[test]
    fn execution_mode_from_str_valid() {
        assert_eq!(
            ExecutionMode::from_str("existing").unwrap(),
            ExecutionMode::Existing
        );
        assert_eq!(
            ExecutionMode::from_str("new_conversation").unwrap(),
            ExecutionMode::NewConversation
        );
    }

    #[test]
    fn execution_mode_from_str_invalid() {
        let err = ExecutionMode::from_str("other").unwrap_err();
        assert!(matches!(err, CronError::InvalidExecutionMode(_)));
    }

    #[test]
    fn execution_mode_as_str_roundtrip() {
        for execution_mode in [ExecutionMode::Existing, ExecutionMode::NewConversation] {
            assert_eq!(
                ExecutionMode::from_str(execution_mode.as_str()).unwrap(),
                execution_mode
            );
        }
    }

    #[test]
    fn created_by_from_str_valid() {
        assert_eq!(CreatedBy::from_str("user").unwrap(), CreatedBy::User);
        assert_eq!(CreatedBy::from_str("agent").unwrap(), CreatedBy::Agent);
    }

    #[test]
    fn created_by_from_str_invalid() {
        let err = CreatedBy::from_str("robot").unwrap_err();
        assert!(matches!(err, CronError::InvalidCreatedBy(_)));
    }

    #[test]
    fn created_by_as_str_roundtrip() {
        for cb in [CreatedBy::User, CreatedBy::Agent] {
            assert_eq!(CreatedBy::from_str(cb.as_str()).unwrap(), cb);
        }
    }

    #[test]
    fn job_status_from_str_all() {
        assert_eq!(JobStatus::from_str("ok").unwrap(), JobStatus::Ok);
        assert_eq!(JobStatus::from_str("error").unwrap(), JobStatus::Error);
        assert_eq!(JobStatus::from_str("skipped").unwrap(), JobStatus::Skipped);
        assert_eq!(JobStatus::from_str("missed").unwrap(), JobStatus::Missed);
        assert_eq!(
            JobStatus::from_str("input_required").unwrap(),
            JobStatus::InputRequired
        );
        assert_eq!(
            JobStatus::from_str("cancelled").unwrap(),
            JobStatus::Cancelled
        );
    }

    #[test]
    fn job_status_from_str_invalid() {
        let err = JobStatus::from_str("running").unwrap_err();
        assert!(matches!(err, CronError::InvalidJobStatus(_)));
    }

    #[test]
    fn job_status_as_str_roundtrip() {
        for s in [
            JobStatus::Ok,
            JobStatus::Error,
            JobStatus::Skipped,
            JobStatus::Missed,
            JobStatus::InputRequired,
            JobStatus::Cancelled,
        ] {
            assert_eq!(JobStatus::from_str(s.as_str()).unwrap(), s);
        }
    }

    // -- Schedule parsing -----------------------------------------------------

    #[test]
    fn parse_schedule_at() {
        let s = parse_schedule("at", "1700000000000", None, Some("once")).unwrap();
        assert_eq!(
            s,
            CronSchedule::At {
                at_ms: 1700000000000,
                description: Some("once".into()),
            }
        );
    }

    #[test]
    fn parse_schedule_at_invalid_value() {
        let err = parse_schedule("at", "not_a_number", None, None).unwrap_err();
        assert!(matches!(err, CronError::InvalidSchedule(_)));
    }

    #[test]
    fn parse_schedule_every() {
        let s = parse_schedule("every", "60000", None, Some("every minute")).unwrap();
        assert_eq!(
            s,
            CronSchedule::Every {
                every_ms: 60000,
                description: Some("every minute".into()),
            }
        );
    }

    #[test]
    fn parse_schedule_every_invalid_value() {
        let err = parse_schedule("every", "abc", None, None).unwrap_err();
        assert!(matches!(err, CronError::InvalidSchedule(_)));
    }

    #[test]
    fn parse_schedule_cron() {
        let s = parse_schedule(
            "cron",
            "0 0 9 * * *",
            Some("Asia/Shanghai"),
            Some("daily 9am"),
        )
        .unwrap();
        assert_eq!(
            s,
            CronSchedule::Cron {
                expr: "0 0 9 * * *".into(),
                tz: Some("Asia/Shanghai".into()),
                description: Some("daily 9am".into()),
            }
        );
    }

    #[test]
    fn parse_schedule_cron_no_tz() {
        let s = parse_schedule("cron", "*/5 * * * *", None, None).unwrap();
        assert_eq!(
            s,
            CronSchedule::Cron {
                expr: "*/5 * * * *".into(),
                tz: None,
                description: None,
            }
        );
    }

    #[test]
    fn parse_schedule_unknown_kind() {
        let err = parse_schedule("weekly", "7", None, None).unwrap_err();
        assert!(matches!(err, CronError::InvalidSchedule(_)));
    }

    fn launch_job() -> CronJob {
        CronJob {
            cron_job_id: JOB_ID.into(),
            user_id: USER_ID.into(),
            name: "Original task".into(),
            enabled: true,
            schedule_revision: 4,
            schedule: CronSchedule::Every {
                every_ms: 60000,
                description: Some("every minute".into()),
            },
            message: "Original user instruction".into(),
            execution_mode: ExecutionMode::NewConversation,
            agent_selection: Some(ProductAgentSelectionDto::Template {
                template_key: "assistant.general".into(),
            }),
            model: Some(AgentChatModelSelectionDto {
                provider_id: PROVIDER_ID.into(),
                model: "selected-model".into(),
            }),
            agent_binding: Some(crate::session_port::test_support::test_binding()),
            agent_label: None,
            effective_model: None,
            clear_context_each_run: false,
            configuration_error: None,
            last_failure: None,
            conversation_id: None,
            conversation_title: None,
            created_by: CreatedBy::User,
            skill_content: None,
            description: Some("keep description".into()),
            created_at: 100,
            updated_at: 200,
            next_run_at: Some(60100),
            last_run_at: None,
            last_run_id: None,
            last_status: None,
            last_error: None,
            run_count: 0,
        }
    }

    #[test]
    fn one_standard_binding_roundtrips_without_runtime_mirrors() {
        let original = launch_job();
        let row = cron_job_to_row(&original).unwrap();
        assert_eq!(
            serde_json::from_str::<AgentBindingValueDto>(row.agent_binding.as_ref().unwrap())
                .unwrap(),
            original.agent_binding.clone().unwrap()
        );
        let restored = cron_job_from_row(row).unwrap();
        assert_eq!(restored, original);
        let response = cron_job_to_response(&restored);
        assert_eq!(response.metadata.agent_binding, original.agent_binding);
        assert_eq!(response.metadata.model, original.model);
        assert_eq!(response.state.configuration_error, None);
        let wire = serde_json::to_value(response).unwrap();
        assert!(wire["metadata"].get("agent_config").is_none());
        assert!(wire["metadata"].get("agent_type").is_none());
        assert!(wire["state"].get("retry_count").is_none());
    }

    #[test]
    fn continuing_session_presentation_never_becomes_stored_authority() {
        let mut job = launch_job();
        job.execution_mode = ExecutionMode::Existing;
        job.conversation_id = Some("0190f5fe-7c00-7a00-8000-000000000091".into());
        job.agent_label = Some("Live canonical Agent".into());
        let row = cron_job_to_row(&job).unwrap();
        assert!(row.agent_binding.is_none());
        assert!(row.agent_selection.is_none());
        assert!(row.model_selection.is_none());
        assert_eq!(
            cron_job_to_response(&job).metadata.agent_label.as_deref(),
            Some("Live canonical Agent")
        );
    }

    #[test]
    fn malformed_typed_authoring_or_binding_json_fails_closed() {
        for field in ["selection", "model", "binding"] {
            let mut row = cron_job_to_row(&launch_job()).unwrap();
            match field {
                "selection" => row.agent_selection = Some("{".into()),
                "model" => row.model_selection = Some("{".into()),
                _ => row.agent_binding = Some("{".into()),
            };
            assert!(cron_job_from_row(row).is_err(), "{field}");
        }
    }

    #[test]
    fn invalid_id_and_enum_rows_are_rejected() {
        for field in ["job", "owner", "session", "mode", "creator", "status"] {
            let mut row = cron_job_to_row(&launch_job()).unwrap();
            match field {
                "job" => row.cron_job_id = "bad".into(),
                "owner" => row.user_id = "bad".into(),
                "session" => row.conversation_id = Some("bad".into()),
                "mode" => row.execution_mode = "bad".into(),
                "creator" => row.created_by = "bad".into(),
                _ => row.last_status = Some("bad".into()),
            };
            assert!(cron_job_from_row(row).is_err(), "{field}");
        }
    }

    #[test]
    fn disabled_unproven_definition_retains_only_authoring_choice_and_diagnostic() {
        let mut job = launch_job();
        job.enabled = false;
        job.agent_binding = None;
        job.configuration_error = Some("CRON_CONFIGURATION_RESELECTION_REQUIRED".into());
        let restored = cron_job_from_row(cron_job_to_row(&job).unwrap()).unwrap();
        assert_eq!(restored, job);
        assert!(restored.model.is_some());
    }

    #[test]
    fn all_schedule_forms_and_timezone_survive_row_domain_and_wire_conversion() {
        for schedule in [
            CronSchedule::At {
                at_ms: 123456,
                description: Some("once".into()),
            },
            CronSchedule::Every {
                every_ms: 30000,
                description: None,
            },
            CronSchedule::Cron {
                expr: "0 0 9 * * *".into(),
                tz: Some("Asia/Shanghai".into()),
                description: Some("daily".into()),
            },
        ] {
            let mut job = launch_job();
            job.schedule = schedule.clone();
            let restored = cron_job_from_row(cron_job_to_row(&job).unwrap()).unwrap();
            assert_eq!(restored.schedule, schedule);
            assert_eq!(
                schedule_from_dto(&cron_job_to_response(&restored).schedule),
                schedule
            );
        }
    }
    #[test]
    fn exact_model_presentation_is_never_written_back_as_an_author_choice() {
        let mut job = launch_job();
        job.model = None;
        job.effective_model = Some(AgentChatModelSelectionDto {
            provider_id: PROVIDER_ID.into(),
            model: "frozen-default".into(),
        });
        job.agent_label = Some("Original frozen Agent".into());
        assert!(cron_job_to_row(&job).unwrap().model_selection.is_none());
        assert_eq!(
            cron_job_to_response(&job)
                .metadata
                .model
                .as_ref()
                .map(|model| model.model.as_str()),
            Some("frozen-default")
        );
        assert_eq!(
            cron_job_to_response(&job).metadata.agent_label.as_deref(),
            Some("Original frozen Agent")
        );
    }
}
