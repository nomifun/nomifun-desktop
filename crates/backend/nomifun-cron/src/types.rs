use std::collections::HashMap;
use std::str::FromStr;

use nomifun_api_types::{
    CronAgentConfigDto, CronJobMetadataDto, CronJobPayloadDto, CronJobResponse, CronJobStateDto,
    CronJobTargetDto, CronScheduleDto,
};
use nomifun_common::TimestampMs;
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

/// Scheduled tasks now run only against agent conversations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    #[default]
    Agent,
}

impl TargetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Agent => "agent",
        }
    }
}

impl FromStr for TargetKind {
    type Err = CronError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "agent" => Ok(Self::Agent),
            "terminal" => Err(CronError::InvalidTargetKind(
                "terminal scheduled tasks are no longer supported".to_owned(),
            )),
            other => Err(CronError::InvalidTargetKind(other.to_owned())),
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
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Error => "error",
            Self::Skipped => "skipped",
            Self::Missed => "missed",
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
            other => Err(CronError::InvalidJobStatus(other.to_owned())),
        }
    }
}

// ---------------------------------------------------------------------------
// Agent configuration (domain model)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CronAgentConfig {
    pub backend: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cli_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_preset: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset_agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_options: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// When `true` and the job reuses an existing conversation
    /// (`ExecutionMode::Existing`), the agent's context is cleared before each
    /// scheduled run so accumulated history does not pile up across ticks.
    /// Visible message records are kept.
    #[serde(default)]
    pub clear_context_each_run: bool,
}

// ---------------------------------------------------------------------------
// CronJob — the core domain model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct CronJob {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub schedule: CronSchedule,
    pub message: String,
    pub execution_mode: ExecutionMode,
    pub agent_config: Option<CronAgentConfig>,
    pub conversation_id: String,
    pub conversation_title: Option<String>,
    pub agent_type: String,
    pub created_by: CreatedBy,
    pub skill_content: Option<String>,
    pub description: Option<String>,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    pub next_run_at: Option<TimestampMs>,
    pub last_run_at: Option<TimestampMs>,
    pub last_status: Option<JobStatus>,
    pub last_error: Option<String>,
    pub run_count: i64,
    pub retry_count: i64,
    pub max_retries: i64,
    /// Execution target kind. Only `Agent` is supported; the field remains for
    /// compatibility with rows/API payloads created before terminal scheduling
    /// support was removed.
    pub target_kind: TargetKind,
}

// ---------------------------------------------------------------------------
// DB → Domain conversion
// ---------------------------------------------------------------------------

pub fn cron_job_from_row(row: CronJobRow) -> Result<CronJob, CronError> {
    let schedule = parse_schedule(
        &row.schedule_kind,
        &row.schedule_value,
        row.schedule_tz.as_deref(),
        row.schedule_description.as_deref(),
    )?;

    let execution_mode = ExecutionMode::from_str(&row.execution_mode)?;
    let created_by = CreatedBy::from_str(&row.created_by)?;

    let agent_config = row
        .agent_config
        .as_deref()
        .map(serde_json::from_str::<CronAgentConfig>)
        .transpose()?;

    let last_status = row
        .last_status
        .as_deref()
        .map(JobStatus::from_str)
        .transpose()?;

    let target_kind = TargetKind::from_str(&row.target_kind)?;

    Ok(CronJob {
        id: row.id,
        name: row.name,
        enabled: row.enabled,
        schedule,
        message: row.payload_message,
        execution_mode,
        agent_config,
        // The DB column is nullable (a `new_conversation` job has no anchor
        // conversation until its first fire) and integer-keyed; the domain model
        // keeps the empty-string convention for "unbound", stringifying the key
        // otherwise.
        conversation_id: row
            .conversation_id
            .map(|id| id.to_string())
            .unwrap_or_default(),
        conversation_title: row.conversation_title,
        agent_type: row.agent_type,
        created_by,
        skill_content: row.skill_content,
        description: row.description,
        created_at: row.created_at,
        updated_at: row.updated_at,
        next_run_at: row.next_run_at,
        last_run_at: row.last_run_at,
        last_status,
        last_error: row.last_error,
        run_count: row.run_count,
        retry_count: row.retry_count,
        max_retries: row.max_retries,
        target_kind,
    })
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
// Domain → DB conversion
// ---------------------------------------------------------------------------

pub fn cron_job_to_row(job: &CronJob) -> Result<CronJobRow, CronError> {
    let (schedule_kind, schedule_value, schedule_tz, schedule_description) =
        schedule_to_row_fields(&job.schedule);

    let agent_config_json = job
        .agent_config
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;

    Ok(CronJobRow {
        id: job.id.clone(),
        name: job.name.clone(),
        enabled: job.enabled,
        schedule_kind,
        schedule_value,
        schedule_tz,
        schedule_description,
        payload_message: job.message.clone(),
        execution_mode: job.execution_mode.as_str().to_owned(),
        agent_config: agent_config_json,
        // Map the empty-string "unbound" convention to a NULL FK so the
        // circular conversations↔cron_jobs FK is satisfied. The column is
        // integer-keyed; a bound id is a positive key. Empty/`0`/non-integer
        // all mean "unbound" → NULL (a `0` FK would violate the FK to
        // conversations, which AUTOINCREMENTs from 1).
        conversation_id: job
            .conversation_id
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|id| *id > 0),
        conversation_title: job.conversation_title.clone(),
        agent_type: job.agent_type.clone(),
        created_by: job.created_by.as_str().to_owned(),
        skill_content: job.skill_content.clone(),
        description: job.description.clone(),
        created_at: job.created_at,
        updated_at: job.updated_at,
        next_run_at: job.next_run_at,
        last_run_at: job.last_run_at,
        last_status: job.last_status.map(|s| s.as_str().to_owned()),
        last_error: job.last_error.clone(),
        run_count: job.run_count,
        retry_count: job.retry_count,
        max_retries: job.max_retries,
        target_kind: job.target_kind.as_str().to_owned(),
        terminal_mode: None,
        terminal_session_id: None,
        terminal_command: None,
        terminal_args: None,
        terminal_script: None,
    })
}

fn schedule_to_row_fields(
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

// ---------------------------------------------------------------------------
// Domain → DTO conversion
// ---------------------------------------------------------------------------

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

    let agent_config_dto = job.agent_config.as_ref().map(|c| CronAgentConfigDto {
        backend: c.backend.clone(),
        name: c.name.clone(),
        cli_path: c.cli_path.clone(),
        is_preset: c.is_preset,
        custom_agent_id: c.custom_agent_id.clone(),
        preset_agent_type: c.preset_agent_type.clone(),
        mode: c.mode.clone(),
        model_id: c.model_id.clone(),
        config_options: c.config_options.clone(),
        workspace: c.workspace.clone(),
        clear_context_each_run: c.clear_context_each_run,
    });

    CronJobResponse {
        id: job.id.clone(),
        name: job.name.clone(),
        description: job.description.clone(),
        enabled: job.enabled,
        schedule,
        target: CronJobTargetDto {
            payload: CronJobPayloadDto::Message {
                text: job.message.clone(),
            },
            execution_mode: Some(job.execution_mode.as_str().to_owned()),
            target_kind: job.target_kind.as_str().to_owned(),
        },
        metadata: CronJobMetadataDto {
            // DTO field is the integer key; the domain holds it as a String
            // (empty == unbound → `0` sentinel on the wire).
            conversation_id: job.conversation_id.parse::<i64>().unwrap_or(0),
            conversation_title: job.conversation_title.clone(),
            agent_type: job.agent_type.clone(),
            created_by: job.created_by.as_str().to_owned(),
            created_at: job.created_at,
            updated_at: job.updated_at,
            agent_config: agent_config_dto,
        },
        state: CronJobStateDto {
            next_run_at_ms: job.next_run_at,
            last_run_at_ms: job.last_run_at,
            last_status: job.last_status.map(|s| s.as_str().to_owned()),
            last_error: job.last_error.clone(),
            run_count: job.run_count,
            retry_count: job.retry_count,
            max_retries: job.max_retries,
        },
    }
}

// ---------------------------------------------------------------------------
// DTO → Domain schedule conversion (for create/update)
// ---------------------------------------------------------------------------

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
        for mode in [ExecutionMode::Existing, ExecutionMode::NewConversation] {
            assert_eq!(ExecutionMode::from_str(mode.as_str()).unwrap(), mode);
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

    // -- DB ↔ Domain roundtrip ------------------------------------------------

    fn sample_row() -> CronJobRow {
        CronJobRow {
            id: "cron_test1".into(),
            name: "Test Job".into(),
            enabled: true,
            schedule_kind: "every".into(),
            schedule_value: "60000".into(),
            schedule_tz: None,
            schedule_description: Some("every minute".into()),
            payload_message: "do something".into(),
            execution_mode: "existing".into(),
            agent_config: Some(r#"{"backend":"acp","name":"Claude"}"#.into()),
            conversation_id: Some(1),
            conversation_title: Some("Test Conv".into()),
            agent_type: "acp".into(),
            created_by: "user".into(),
            skill_content: Some("---\nname: test\n---\nContent".into()),
            description: None,
            created_at: 1000,
            updated_at: 2000,
            next_run_at: Some(3000),
            last_run_at: Some(1500),
            last_status: Some("ok".into()),
            last_error: None,
            run_count: 5,
            retry_count: 0,
            max_retries: 3,
            target_kind: "agent".into(),
            terminal_mode: None,
            terminal_session_id: None,
            terminal_command: None,
            terminal_args: None,
            terminal_script: None,
        }
    }

    fn sample_job() -> CronJob {
        CronJob {
            id: "cron_test1".into(),
            name: "Test Job".into(),
            enabled: true,
            schedule: CronSchedule::Every {
                every_ms: 60000,
                description: Some("every minute".into()),
            },
            message: "do something".into(),
            execution_mode: ExecutionMode::Existing,
            agent_config: Some(CronAgentConfig {
                backend: "acp".into(),
                name: "Claude".into(),
                cli_path: None,
                is_preset: None,
                custom_agent_id: None,
                preset_agent_type: None,
                mode: None,
                model_id: None,
                config_options: None,
                workspace: None,
                clear_context_each_run: false,
            }),
            conversation_id: "1".into(),
            conversation_title: Some("Test Conv".into()),
            agent_type: "acp".into(),
            created_by: CreatedBy::User,
            skill_content: Some("---\nname: test\n---\nContent".into()),
            description: None,
            created_at: 1000,
            updated_at: 2000,
            next_run_at: Some(3000),
            last_run_at: Some(1500),
            last_status: Some(JobStatus::Ok),
            last_error: None,
            run_count: 5,
            retry_count: 0,
            max_retries: 3,
            target_kind: TargetKind::Agent,
        }
    }

    #[test]
    fn row_to_domain_roundtrip() {
        let row = sample_row();
        let job = cron_job_from_row(row).unwrap();
        assert_eq!(job.id, "cron_test1");
        assert_eq!(job.name, "Test Job");
        assert!(job.enabled);
        assert_eq!(
            job.schedule,
            CronSchedule::Every {
                every_ms: 60000,
                description: Some("every minute".into()),
            }
        );
        assert_eq!(job.execution_mode, ExecutionMode::Existing);
        assert_eq!(job.created_by, CreatedBy::User);
        assert_eq!(job.last_status, Some(JobStatus::Ok));
        assert!(job.agent_config.is_some());
        assert_eq!(job.agent_config.as_ref().unwrap().backend, "acp");
    }

    #[test]
    fn domain_to_row_roundtrip() {
        let job = sample_job();
        let row = cron_job_to_row(&job).unwrap();
        assert_eq!(row.id, "cron_test1");
        assert_eq!(row.schedule_kind, "every");
        assert_eq!(row.schedule_value, "60000");
        assert_eq!(row.execution_mode, "existing");
        assert_eq!(row.created_by, "user");
        assert_eq!(row.last_status.as_deref(), Some("ok"));
        assert!(row.agent_config.is_some());

        let restored = cron_job_from_row(row).unwrap();
        assert_eq!(restored.id, job.id);
        assert_eq!(restored.schedule, job.schedule);
        assert_eq!(restored.execution_mode, job.execution_mode);
        assert_eq!(restored.created_by, job.created_by);
        assert_eq!(restored.last_status, job.last_status);
    }

    #[test]
    fn row_to_domain_at_type() {
        let row = CronJobRow {
            schedule_kind: "at".into(),
            schedule_value: "1700000000000".into(),
            schedule_tz: None,
            schedule_description: Some("once".into()),
            ..sample_row()
        };
        let job = cron_job_from_row(row).unwrap();
        assert_eq!(
            job.schedule,
            CronSchedule::At {
                at_ms: 1700000000000,
                description: Some("once".into()),
            }
        );
    }

    #[test]
    fn row_to_domain_cron_type_with_tz() {
        let row = CronJobRow {
            schedule_kind: "cron".into(),
            schedule_value: "0 0 9 * * *".into(),
            schedule_tz: Some("Asia/Shanghai".into()),
            schedule_description: Some("daily 9am".into()),
            ..sample_row()
        };
        let job = cron_job_from_row(row).unwrap();
        assert_eq!(
            job.schedule,
            CronSchedule::Cron {
                expr: "0 0 9 * * *".into(),
                tz: Some("Asia/Shanghai".into()),
                description: Some("daily 9am".into()),
            }
        );
    }

    #[test]
    fn row_to_domain_no_optional_fields() {
        let row = CronJobRow {
            agent_config: None,
            conversation_title: None,
            skill_content: None,
            next_run_at: None,
            last_run_at: None,
            last_status: None,
            last_error: None,
            ..sample_row()
        };
        let job = cron_job_from_row(row).unwrap();
        assert!(job.agent_config.is_none());
        assert!(job.conversation_title.is_none());
        assert!(job.last_status.is_none());
    }

    #[test]
    fn row_to_domain_invalid_execution_mode() {
        let row = CronJobRow {
            execution_mode: "parallel".into(),
            ..sample_row()
        };
        let err = cron_job_from_row(row).unwrap_err();
        assert!(matches!(err, CronError::InvalidExecutionMode(_)));
    }

    #[test]
    fn row_to_domain_invalid_created_by() {
        let row = CronJobRow {
            created_by: "system".into(),
            ..sample_row()
        };
        let err = cron_job_from_row(row).unwrap_err();
        assert!(matches!(err, CronError::InvalidCreatedBy(_)));
    }

    #[test]
    fn row_to_domain_invalid_status() {
        let row = CronJobRow {
            last_status: Some("running".into()),
            ..sample_row()
        };
        let err = cron_job_from_row(row).unwrap_err();
        assert!(matches!(err, CronError::InvalidJobStatus(_)));
    }

    #[test]
    fn row_to_domain_invalid_agent_config_json() {
        let row = CronJobRow {
            agent_config: Some("not json".into()),
            ..sample_row()
        };
        let err = cron_job_from_row(row).unwrap_err();
        assert!(matches!(err, CronError::Json(_)));
    }

    // -- Domain → DTO ---------------------------------------------------------

    #[test]
    fn domain_to_dto_every() {
        let job = sample_job();
        let resp = cron_job_to_response(&job);
        assert_eq!(resp.id, "cron_test1");
        assert_eq!(resp.name, "Test Job");
        assert!(resp.enabled);
        assert!(matches!(
            resp.schedule,
            CronScheduleDto::Every {
                every_ms: 60000,
                ..
            }
        ));
        assert_eq!(resp.target.execution_mode.as_deref(), Some("existing"));
        assert_eq!(resp.metadata.conversation_id, 1);
        assert_eq!(resp.metadata.agent_type, "acp");
        assert_eq!(resp.metadata.created_by, "user");
        assert_eq!(resp.state.run_count, 5);
        assert_eq!(resp.state.last_status.as_deref(), Some("ok"));
    }

    #[test]
    fn domain_to_dto_at_type() {
        let job = CronJob {
            schedule: CronSchedule::At {
                at_ms: 1700000000000,
                description: Some("once".into()),
            },
            ..sample_job()
        };
        let resp = cron_job_to_response(&job);
        assert!(matches!(
            resp.schedule,
            CronScheduleDto::At {
                at_ms: 1700000000000,
                ..
            }
        ));
    }

    #[test]
    fn domain_to_dto_cron_type() {
        let job = CronJob {
            schedule: CronSchedule::Cron {
                expr: "0 0 9 * * *".into(),
                tz: Some("UTC".into()),
                description: Some("daily".into()),
            },
            ..sample_job()
        };
        let resp = cron_job_to_response(&job);
        assert!(matches!(resp.schedule, CronScheduleDto::Cron { .. }));
    }

    #[test]
    fn domain_to_dto_no_agent_config() {
        let job = CronJob {
            agent_config: None,
            ..sample_job()
        };
        let resp = cron_job_to_response(&job);
        assert!(resp.metadata.agent_config.is_none());
    }

    #[test]
    fn domain_to_dto_new_conversation_mode() {
        let job = CronJob {
            execution_mode: ExecutionMode::NewConversation,
            ..sample_job()
        };
        let resp = cron_job_to_response(&job);
        assert_eq!(
            resp.target.execution_mode.as_deref(),
            Some("new_conversation")
        );
    }

    // -- DTO → Domain schedule ------------------------------------------------

    #[test]
    fn schedule_from_dto_at() {
        let dto = CronScheduleDto::At {
            at_ms: 5000,
            description: Some("once".into()),
        };
        let s = schedule_from_dto(&dto);
        assert_eq!(
            s,
            CronSchedule::At {
                at_ms: 5000,
                description: Some("once".into()),
            }
        );
    }

    #[test]
    fn schedule_from_dto_every() {
        let dto = CronScheduleDto::Every {
            every_ms: 30000,
            description: None,
        };
        let s = schedule_from_dto(&dto);
        assert_eq!(
            s,
            CronSchedule::Every {
                every_ms: 30000,
                description: None,
            }
        );
    }

    #[test]
    fn schedule_from_dto_cron() {
        let dto = CronScheduleDto::Cron {
            expr: "0 */5 * * * *".into(),
            tz: Some("UTC".into()),
            description: Some("every 5m".into()),
        };
        let s = schedule_from_dto(&dto);
        assert_eq!(
            s,
            CronSchedule::Cron {
                expr: "0 */5 * * * *".into(),
                tz: Some("UTC".into()),
                description: Some("every 5m".into()),
            }
        );
    }

    // -- Target kind ----------------------------------------------------------

    #[test]
    fn target_kind_accepts_agent_only() {
        assert_eq!(
            TargetKind::from_str(TargetKind::Agent.as_str()).unwrap(),
            TargetKind::Agent
        );
        assert!(TargetKind::from_str("bogus").is_err());
        assert!(matches!(
            TargetKind::from_str("terminal"),
            Err(CronError::InvalidTargetKind(_))
        ));
    }

    #[test]
    fn agent_job_clears_legacy_terminal_columns_after_roundtrip() {
        let row = cron_job_to_row(&sample_job()).unwrap();
        assert_eq!(row.target_kind, "agent");
        assert!(row.terminal_mode.is_none());
        assert!(row.terminal_session_id.is_none());
        assert!(row.terminal_command.is_none());
        assert!(row.terminal_args.is_none());
        assert!(row.terminal_script.is_none());
        let restored = cron_job_from_row(row).unwrap();
        assert_eq!(restored.target_kind, TargetKind::Agent);
    }
}
