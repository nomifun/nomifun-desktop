use std::collections::HashMap;

use nomifun_common::TimestampMs;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// A. Schedule — tagged union with three variants
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum CronScheduleDto {
    #[serde(rename = "at")]
    At {
        at_ms: TimestampMs,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    #[serde(rename = "every")]
    Every {
        every_ms: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    #[serde(rename = "cron")]
    Cron {
        expr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tz: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// B. Agent configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronAgentConfigDto {
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
    /// Clear the agent context before each scheduled run (only meaningful when
    /// the job reuses an existing conversation). Defaults to `false`.
    #[serde(default)]
    pub clear_context_each_run: bool,
}

// ---------------------------------------------------------------------------
// C. CronJob response — nested structure matching API Spec
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum CronJobPayloadDto {
    #[serde(rename = "message")]
    Message { text: String },
}

fn default_target_kind() -> String {
    "agent".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronJobTargetDto {
    pub payload: CronJobPayloadDto,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<String>,
    /// Only `"agent"` is supported. Kept on the wire so older clients sending
    /// `"terminal"` receive a precise service-layer rejection instead of being
    /// silently treated as an agent task.
    #[serde(default = "default_target_kind")]
    pub target_kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronJobMetadataDto {
    pub conversation_id: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_title: Option<String>,
    pub agent_type: String,
    pub created_by: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_config: Option<CronAgentConfigDto>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronJobStateDto {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_run_at_ms: Option<TimestampMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at_ms: Option<TimestampMs>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub run_count: i64,
    pub retry_count: i64,
    pub max_retries: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronJobResponse {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub enabled: bool,
    pub schedule: CronScheduleDto,
    pub target: CronJobTargetDto,
    pub metadata: CronJobMetadataDto,
    pub state: CronJobStateDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronJobRunResponse {
    pub id: String,
    pub job_id: String,
    pub executed_at_ms: TimestampMs,
    pub status: String,
}

// ---------------------------------------------------------------------------
// D. Create / Update request DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct CreateCronJobRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub schedule: CronScheduleDto,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    pub conversation_id: i64,
    #[serde(default)]
    pub conversation_title: Option<String>,
    pub agent_type: String,
    pub created_by: String,
    #[serde(default)]
    pub execution_mode: Option<String>,
    #[serde(default)]
    pub agent_config: Option<CronAgentConfigDto>,
    /// Only `"agent"` is supported. Non-agent values are rejected by the cron
    /// service for compatibility with older clients.
    #[serde(default = "default_target_kind")]
    pub target_kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateCronJobRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub schedule: Option<CronScheduleDto>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub execution_mode: Option<String>,
    #[serde(default)]
    pub agent_config: Option<CronAgentConfigDto>,
    #[serde(default)]
    pub conversation_title: Option<String>,
    #[serde(default)]
    pub max_retries: Option<i64>,
    /// Only `"agent"` is supported. `None` keeps the current target kind.
    #[serde(default)]
    pub target_kind: Option<String>,
}

// ---------------------------------------------------------------------------
// E. Query parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ListCronJobsQuery {
    pub conversation_id: Option<i64>,
}

// ---------------------------------------------------------------------------
// F. Other responses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunNowResponse {
    pub conversation_id: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SaveCronSkillRequest {
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HasSkillResponse {
    pub has_skill: bool,
}

// ---------------------------------------------------------------------------
// G. Event payloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronJobExecutedEvent {
    pub job_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronJobRemovedPayload {
    pub job_id: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- A. CronScheduleDto ---------------------------------------------------

    #[test]
    fn schedule_at_serialize() {
        let s = CronScheduleDto::At {
            at_ms: 1700000000000,
            description: Some("Run at specific time".into()),
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["kind"], "at");
        assert_eq!(json["at_ms"], 1700000000000_i64);
        assert_eq!(json["description"], "Run at specific time");
    }

    #[test]
    fn schedule_at_deserialize() {
        let raw = json!({"kind": "at", "at_ms": 1700000000000_i64, "description": "once"});
        let s: CronScheduleDto = serde_json::from_value(raw).unwrap();
        assert_eq!(
            s,
            CronScheduleDto::At {
                at_ms: 1700000000000,
                description: Some("once".into()),
            }
        );
    }

    #[test]
    fn schedule_at_without_description() {
        let raw = json!({"kind": "at", "at_ms": 1000});
        let s: CronScheduleDto = serde_json::from_value(raw).unwrap();
        assert_eq!(
            s,
            CronScheduleDto::At {
                at_ms: 1000,
                description: None,
            }
        );
        let json = serde_json::to_value(&s).unwrap();
        assert!(json.get("description").is_none());
    }

    #[test]
    fn schedule_every_serialize() {
        let s = CronScheduleDto::Every {
            every_ms: 60000,
            description: Some("Every minute".into()),
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["kind"], "every");
        assert_eq!(json["every_ms"], 60000);
        assert_eq!(json["description"], "Every minute");
    }

    #[test]
    fn schedule_every_deserialize() {
        let raw = json!({"kind": "every", "every_ms": 300000});
        let s: CronScheduleDto = serde_json::from_value(raw).unwrap();
        assert_eq!(
            s,
            CronScheduleDto::Every {
                every_ms: 300000,
                description: None,
            }
        );
    }

    #[test]
    fn schedule_cron_serialize() {
        let s = CronScheduleDto::Cron {
            expr: "0 0 9 * * *".into(),
            tz: Some("Asia/Shanghai".into()),
            description: Some("Daily at 9am".into()),
        };
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["kind"], "cron");
        assert_eq!(json["expr"], "0 0 9 * * *");
        assert_eq!(json["tz"], "Asia/Shanghai");
        assert_eq!(json["description"], "Daily at 9am");
    }

    #[test]
    fn schedule_cron_without_tz() {
        let raw = json!({"kind": "cron", "expr": "0 */5 * * * *"});
        let s: CronScheduleDto = serde_json::from_value(raw).unwrap();
        assert_eq!(
            s,
            CronScheduleDto::Cron {
                expr: "0 */5 * * * *".into(),
                tz: None,
                description: None,
            }
        );
    }

    #[test]
    fn schedule_invalid_kind() {
        let raw = json!({"kind": "unknown", "value": 123});
        let result = serde_json::from_value::<CronScheduleDto>(raw);
        assert!(result.is_err());
    }

    #[test]
    fn schedule_at_missing_at_ms() {
        let raw = json!({"kind": "at"});
        let result = serde_json::from_value::<CronScheduleDto>(raw);
        assert!(result.is_err());
    }

    #[test]
    fn schedule_every_missing_every_ms() {
        let raw = json!({"kind": "every"});
        let result = serde_json::from_value::<CronScheduleDto>(raw);
        assert!(result.is_err());
    }

    #[test]
    fn schedule_cron_missing_expr() {
        let raw = json!({"kind": "cron"});
        let result = serde_json::from_value::<CronScheduleDto>(raw);
        assert!(result.is_err());
    }

    #[test]
    fn schedule_roundtrip_all_variants() {
        let variants = vec![
            CronScheduleDto::At {
                at_ms: 999,
                description: Some("once".into()),
            },
            CronScheduleDto::Every {
                every_ms: 5000,
                description: None,
            },
            CronScheduleDto::Cron {
                expr: "* * * * *".into(),
                tz: Some("UTC".into()),
                description: Some("every minute".into()),
            },
        ];
        for v in &variants {
            let json = serde_json::to_string(v).unwrap();
            let parsed: CronScheduleDto = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, v);
        }
    }

    // -- B. CronAgentConfigDto ------------------------------------------------

    #[test]
    fn agent_config_full() {
        let raw = json!({
            "backend": "acp",
            "name": "Claude Agent",
            "cli_path": "/usr/bin/claude",
            "is_preset": true,
            "custom_agent_id": "agent-1",
            "preset_agent_type": "claude",
            "mode": "auto",
            "model_id": "claude-sonnet-4-6",
            "config_options": {"key": "value"},
            "workspace": "/tmp/ws"
        });
        let c: CronAgentConfigDto = serde_json::from_value(raw).unwrap();
        assert_eq!(c.backend, "acp");
        assert_eq!(c.name, "Claude Agent");
        assert_eq!(c.cli_path.as_deref(), Some("/usr/bin/claude"));
        assert_eq!(c.is_preset, Some(true));
        assert_eq!(c.custom_agent_id.as_deref(), Some("agent-1"));
        assert_eq!(c.model_id.as_deref(), Some("claude-sonnet-4-6"));
        assert_eq!(c.config_options.as_ref().unwrap()["key"], "value");
    }

    #[test]
    fn agent_config_minimal() {
        let raw = json!({"backend": "openai", "name": "GPT"});
        let c: CronAgentConfigDto = serde_json::from_value(raw).unwrap();
        assert_eq!(c.backend, "openai");
        assert_eq!(c.name, "GPT");
        assert!(c.cli_path.is_none());
        assert!(c.is_preset.is_none());
        assert!(c.config_options.is_none());
    }

    #[test]
    fn agent_config_serialize_omits_none() {
        let c = CronAgentConfigDto {
            backend: "acp".into(),
            name: "Test".into(),
            cli_path: None,
            is_preset: None,
            custom_agent_id: None,
            preset_agent_type: None,
            mode: None,
            model_id: None,
            config_options: None,
            workspace: None,
            clear_context_each_run: false,
        };
        let json = serde_json::to_value(&c).unwrap();
        assert!(json.get("cli_path").is_none());
        assert!(json.get("is_preset").is_none());
        assert!(json.get("config_options").is_none());
    }

    #[test]
    fn agent_config_roundtrip() {
        let c = CronAgentConfigDto {
            backend: "acp".into(),
            name: "Agent".into(),
            cli_path: Some("/bin/x".into()),
            is_preset: Some(false),
            custom_agent_id: Some("c1".into()),
            preset_agent_type: None,
            mode: Some("plan".into()),
            model_id: Some("m1".into()),
            config_options: Some(HashMap::from([("a".into(), "b".into())])),
            workspace: Some("/ws".into()),
            clear_context_each_run: false,
        };
        let json = serde_json::to_string(&c).unwrap();
        let parsed: CronAgentConfigDto = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, c);
    }

    // -- C. CronJobResponse ---------------------------------------------------

    fn sample_cron_job_response() -> CronJobResponse {
        CronJobResponse {
            id: "cron_abc".into(),
            name: "Daily report".into(),
            description: Some("Daily report description".into()),
            enabled: true,
            schedule: CronScheduleDto::Cron {
                expr: "0 0 9 * * *".into(),
                tz: Some("Asia/Shanghai".into()),
                description: Some("Daily at 9am".into()),
            },
            target: CronJobTargetDto {
                payload: CronJobPayloadDto::Message {
                    text: "Generate report".into(),
                },
                execution_mode: Some("new_conversation".into()),
                target_kind: "agent".into(),
            },
            metadata: CronJobMetadataDto {
                conversation_id: 1,
                conversation_title: Some("Reports".into()),
                agent_type: "acp".into(),
                created_by: "user".into(),
                created_at: 1700000000000,
                updated_at: 1700001000000,
                agent_config: Some(CronAgentConfigDto {
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
            },
            state: CronJobStateDto {
                next_run_at_ms: Some(1700100000000),
                last_run_at_ms: Some(1700000000000),
                last_status: Some("ok".into()),
                last_error: None,
                run_count: 5,
                retry_count: 0,
                max_retries: 3,
            },
        }
    }

    #[test]
    fn cron_job_response_serialize_snake_case() {
        let resp = sample_cron_job_response();
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["id"], "cron_abc");
        assert_eq!(json["name"], "Daily report");
        assert_eq!(json["enabled"], true);
        assert_eq!(json["schedule"]["kind"], "cron");
        assert_eq!(json["schedule"]["expr"], "0 0 9 * * *");
        assert_eq!(json["target"]["payload"]["kind"], "message");
        assert_eq!(json["target"]["payload"]["text"], "Generate report");
        assert_eq!(json["target"]["execution_mode"], "new_conversation");
        assert_eq!(json["metadata"]["conversation_id"], 1);
        assert_eq!(json["metadata"]["agent_type"], "acp");
        assert_eq!(json["metadata"]["created_by"], "user");
        assert_eq!(json["metadata"]["created_at"], 1700000000000_i64);
        assert_eq!(json["state"]["next_run_at_ms"], 1700100000000_i64);
        assert_eq!(json["state"]["last_status"], "ok");
        assert_eq!(json["state"]["run_count"], 5);
        assert_eq!(json["state"]["retry_count"], 0);
        assert_eq!(json["state"]["max_retries"], 3);
        assert!(json["state"].get("last_error").is_none());
    }

    #[test]
    fn cron_job_response_roundtrip() {
        let resp = sample_cron_job_response();
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: CronJobResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, resp);
    }

    #[test]
    fn cron_job_response_minimal_state() {
        let resp = CronJobResponse {
            id: "cron_min".into(),
            name: "Ping".into(),
            description: None,
            enabled: false,
            schedule: CronScheduleDto::Every {
                every_ms: 60000,
                description: None,
            },
            target: CronJobTargetDto {
                payload: CronJobPayloadDto::Message {
                    text: "ping".into(),
                },
                execution_mode: None,
                target_kind: "agent".into(),
            },
            metadata: CronJobMetadataDto {
                conversation_id: 2,
                conversation_title: None,
                agent_type: "gemini".into(),
                created_by: "agent".into(),
                created_at: 1000,
                updated_at: 1000,
                agent_config: None,
            },
            state: CronJobStateDto {
                next_run_at_ms: None,
                last_run_at_ms: None,
                last_status: None,
                last_error: None,
                run_count: 0,
                retry_count: 0,
                max_retries: 3,
            },
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["target"].get("execution_mode").is_none());
        assert!(json["metadata"].get("conversation_title").is_none());
        assert!(json["metadata"].get("agent_config").is_none());
        assert!(json["state"].get("next_run_at_ms").is_none());
        assert!(json["state"].get("last_status").is_none());
    }

    // -- D. CreateCronJobRequest ----------------------------------------------

    #[test]
    fn create_request_full() {
        let raw = json!({
            "name": "Daily task",
            "schedule": {"kind": "cron", "expr": "0 0 9 * * *", "tz": "UTC"},
            "message": "Do the thing",
            "conversation_id": 1,
            "conversation_title": "Tasks",
            "agent_type": "acp",
            "created_by": "user",
            "execution_mode": "new_conversation",
            "agent_config": {"backend": "acp", "name": "Claude"}
        });
        let req: CreateCronJobRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.name, "Daily task");
        assert_eq!(req.message.as_deref(), Some("Do the thing"));
        assert_eq!(req.conversation_id, 1);
        assert_eq!(req.agent_type, "acp");
        assert_eq!(req.created_by, "user");
        assert_eq!(req.execution_mode.as_deref(), Some("new_conversation"));
        assert!(req.agent_config.is_some());
    }

    #[test]
    fn create_request_minimal() {
        let raw = json!({
            "name": "Ping",
            "schedule": {"kind": "every", "every_ms": 60000},
            "conversation_id": 1,
            "agent_type": "acp",
            "created_by": "agent"
        });
        let req: CreateCronJobRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.name, "Ping");
        assert!(req.message.is_none());
        assert!(req.prompt.is_none());
        assert!(req.execution_mode.is_none());
        assert!(req.agent_config.is_none());
    }

    #[test]
    fn create_request_with_prompt() {
        let raw = json!({
            "name": "Task",
            "schedule": {"kind": "at", "at_ms": 1000},
            "prompt": "Do something",
            "conversation_id": 1,
            "agent_type": "gemini",
            "created_by": "user"
        });
        let req: CreateCronJobRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.prompt.as_deref(), Some("Do something"));
        assert!(req.message.is_none());
    }

    #[test]
    fn create_request_missing_name() {
        let raw = json!({
            "schedule": {"kind": "every", "every_ms": 1000},
            "conversation_id": 1,
            "agent_type": "acp",
            "created_by": "user"
        });
        assert!(serde_json::from_value::<CreateCronJobRequest>(raw).is_err());
    }

    #[test]
    fn create_request_missing_schedule() {
        let raw = json!({
            "name": "X",
            "conversation_id": 1,
            "agent_type": "acp",
            "created_by": "user"
        });
        assert!(serde_json::from_value::<CreateCronJobRequest>(raw).is_err());
    }

    #[test]
    fn create_request_missing_conversation_id() {
        let raw = json!({
            "name": "X",
            "schedule": {"kind": "every", "every_ms": 1000},
            "agent_type": "acp",
            "created_by": "user"
        });
        assert!(serde_json::from_value::<CreateCronJobRequest>(raw).is_err());
    }

    #[test]
    fn create_request_missing_agent_type() {
        let raw = json!({
            "name": "X",
            "schedule": {"kind": "every", "every_ms": 1000},
            "conversation_id": 1,
            "created_by": "user"
        });
        assert!(serde_json::from_value::<CreateCronJobRequest>(raw).is_err());
    }

    #[test]
    fn create_request_missing_created_by() {
        let raw = json!({
            "name": "X",
            "schedule": {"kind": "every", "every_ms": 1000},
            "conversation_id": 1,
            "agent_type": "acp"
        });
        assert!(serde_json::from_value::<CreateCronJobRequest>(raw).is_err());
    }

    // -- E. UpdateCronJobRequest ----------------------------------------------

    #[test]
    fn update_request_partial() {
        let raw = json!({"name": "New name", "enabled": false});
        let req: UpdateCronJobRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.name.as_deref(), Some("New name"));
        assert_eq!(req.enabled, Some(false));
        assert!(req.schedule.is_none());
        assert!(req.message.is_none());
        assert!(req.max_retries.is_none());
    }

    #[test]
    fn update_request_schedule_change() {
        let raw = json!({
            "schedule": {"kind": "cron", "expr": "0 */10 * * * *"}
        });
        let req: UpdateCronJobRequest = serde_json::from_value(raw).unwrap();
        assert!(req.schedule.is_some());
        assert!(req.name.is_none());
    }

    #[test]
    fn update_request_empty() {
        let raw = json!({});
        let req: UpdateCronJobRequest = serde_json::from_value(raw).unwrap();
        assert!(req.name.is_none());
        assert!(req.enabled.is_none());
        assert!(req.schedule.is_none());
        assert!(req.message.is_none());
        assert!(req.execution_mode.is_none());
        assert!(req.max_retries.is_none());
    }

    #[test]
    fn update_request_with_max_retries() {
        let raw = json!({"max_retries": 5});
        let req: UpdateCronJobRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.max_retries, Some(5));
    }

    // -- F. ListCronJobsQuery -------------------------------------------------

    #[test]
    fn list_query_with_conversation_id() {
        let raw = json!({"conversation_id": 1});
        let q: ListCronJobsQuery = serde_json::from_value(raw).unwrap();
        assert_eq!(q.conversation_id, Some(1));
    }

    #[test]
    fn list_query_empty() {
        let raw = json!({});
        let q: ListCronJobsQuery = serde_json::from_value(raw).unwrap();
        assert!(q.conversation_id.is_none());
    }

    // -- G. RunNowResponse / HasSkillResponse / SaveCronSkillRequest ----------

    #[test]
    fn run_now_response_serialize() {
        let r = RunNowResponse {
            conversation_id: 99,
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["conversation_id"], 99);
    }

    #[test]
    fn run_now_response_roundtrip() {
        let r = RunNowResponse { conversation_id: 1 };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: RunNowResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn has_skill_response_true() {
        let r = HasSkillResponse { has_skill: true };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["has_skill"], true);
    }

    #[test]
    fn has_skill_response_false() {
        let r = HasSkillResponse { has_skill: false };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["has_skill"], false);
    }

    #[test]
    fn has_skill_response_roundtrip() {
        let r = HasSkillResponse { has_skill: true };
        let json = serde_json::to_string(&r).unwrap();
        let parsed: HasSkillResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, r);
    }

    #[test]
    fn save_skill_request_deserialize() {
        let raw = json!({"content": "---\nname: test\n---\nDo something"});
        let req: SaveCronSkillRequest = serde_json::from_value(raw).unwrap();
        assert_eq!(req.content, "---\nname: test\n---\nDo something");
    }

    #[test]
    fn save_skill_request_missing_content() {
        let raw = json!({});
        assert!(serde_json::from_value::<SaveCronSkillRequest>(raw).is_err());
    }

    // -- H. Event payloads ----------------------------------------------------

    #[test]
    fn executed_event_serialize() {
        let e = CronJobExecutedEvent {
            job_id: "cron_1".into(),
            status: "ok".into(),
            error: None,
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["job_id"], "cron_1");
        assert_eq!(json["status"], "ok");
        assert!(json.get("error").is_none());
    }

    #[test]
    fn executed_event_with_error() {
        let e = CronJobExecutedEvent {
            job_id: "cron_2".into(),
            status: "error".into(),
            error: Some("timeout".into()),
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["status"], "error");
        assert_eq!(json["error"], "timeout");
    }

    #[test]
    fn executed_event_roundtrip() {
        let e = CronJobExecutedEvent {
            job_id: "cron_1".into(),
            status: "skipped".into(),
            error: Some("busy".into()),
        };
        let json = serde_json::to_string(&e).unwrap();
        let parsed: CronJobExecutedEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, e);
    }

    #[test]
    fn removed_payload_serialize() {
        let p = CronJobRemovedPayload {
            job_id: "cron_del".into(),
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["job_id"], "cron_del");
    }

    #[test]
    fn removed_payload_roundtrip() {
        let p = CronJobRemovedPayload {
            job_id: "cron_x".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let parsed: CronJobRemovedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, p);
    }

    // -- Payload DTO ----------------------------------------------------------

    #[test]
    fn payload_message_serialize() {
        let p = CronJobPayloadDto::Message {
            text: "hello".into(),
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["kind"], "message");
        assert_eq!(json["text"], "hello");
    }

    #[test]
    fn payload_message_roundtrip() {
        let p = CronJobPayloadDto::Message {
            text: "test".into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let parsed: CronJobPayloadDto = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, p);
    }
}
