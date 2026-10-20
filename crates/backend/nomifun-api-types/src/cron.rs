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
// Agent authoring metadata and the sole frozen execution contract
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CronJobMetadataDto {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::serde_util::deserialize_optional_conversation_id"
    )]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_title: Option<String>,
    pub created_by: String,
    pub created_at: TimestampMs,
    pub updated_at: TimestampMs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_selection: Option<crate::ProductAgentSelectionDto>,
    /// Effective model from the frozen binding or bound Session. Disabled
    /// definitions without a binding retain their configured authoring choice.
    /// This display field never selects scheduled execution authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<crate::AgentChatModelSelectionDto>,
    /// Read-only canonical binding presentation. A continuing Session retains
    /// its sole authority; this projection is never a second persisted binding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_binding: Option<crate::AgentBindingValueDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_label: Option<String>,
    #[serde(default)]
    pub clear_context_each_run: bool,
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
    /// Read-only projection of the exact run's canonical failure, never a
    /// second copy of the Session error fact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<crate::AgentStreamErrorData>,
    pub run_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configuration_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CronJobResponse {
    #[serde(deserialize_with = "crate::serde_util::deserialize_cron_job_id")]
    pub cron_job_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub enabled: bool,
    pub schedule: CronScheduleDto,
    pub message: String,
    pub execution_mode: String,
    pub metadata: CronJobMetadataDto,
    pub state: CronJobStateDto,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CronJobRunResponse {
    #[serde(deserialize_with = "crate::serde_util::deserialize_cron_job_run_id")]
    pub cron_job_run_id: String,
    #[serde(deserialize_with = "crate::serde_util::deserialize_cron_job_id")]
    pub cron_job_id: String,
    pub executed_at_ms: TimestampMs,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<crate::AgentStreamErrorData>,
}

// ---------------------------------------------------------------------------
// D. Create / Update request DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCronJobRequest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub schedule: CronScheduleDto,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_conversation_id"
    )]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub conversation_title: Option<String>,
    pub created_by: String,
    #[serde(default)]
    pub execution_mode: Option<String>,
    #[serde(default)]
    pub agent_selection: Option<crate::ProductAgentSelectionDto>,
    #[serde(default)]
    pub model: Option<crate::AgentChatModelSelectionDto>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub resource_selections: Vec<crate::AgentResourceSelectionDto>,
    #[serde(default)]
    pub clear_context_each_run: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub agent_selection: Option<crate::ProductAgentSelectionDto>,
    #[serde(default)]
    pub model: Option<crate::AgentChatModelSelectionDto>,
    #[serde(default)]
    pub workspace: Option<String>,
    #[serde(default)]
    pub resource_selections: Option<Vec<crate::AgentResourceSelectionDto>>,
    #[serde(default)]
    pub clear_context_each_run: Option<bool>,
    #[serde(default)]
    pub conversation_title: Option<String>,
}

// ---------------------------------------------------------------------------
// E. Query parameters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ListCronJobsQuery {
    #[serde(
        default,
        deserialize_with = "crate::serde_util::deserialize_optional_conversation_id"
    )]
    pub conversation_id: Option<String>,
}

// ---------------------------------------------------------------------------
// F. Other responses
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunNowResponse {
    #[serde(deserialize_with = "crate::serde_util::deserialize_conversation_id")]
    pub conversation_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct CronJobExecutedEvent {
    #[serde(deserialize_with = "crate::serde_util::deserialize_cron_job_id")]
    pub cron_job_id: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CronJobRemovedPayload {
    #[serde(deserialize_with = "crate::serde_util::deserialize_cron_job_id")]
    pub cron_job_id: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const JOB_ID: &str = "0190f5fe-7c00-7a00-8abc-012345678941";
    const JOB_ID_2: &str = "0190f5fe-7c00-7a00-8abc-012345678942";

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

    fn create_body() -> serde_json::Value {
        json!({
            "name":"Weather", "schedule":{"kind":"every","every_ms":60000},
            "message":"Query the weather", "created_by":"user",
            "agent_selection":{"kind":"template","template_key":"assistant.general"},
            "model":{"provider_id":"0190f5fe-7c00-7a00-8000-000000000001","model":"step-5-preview"}
        })
    }

    #[test]
    fn workbench_selection_and_model_are_independent_authoring_inputs() {
        let request: CreateCronJobRequest = serde_json::from_value(create_body()).unwrap();
        assert!(matches!(request.agent_selection, Some(crate::ProductAgentSelectionDto::Template { ref template_key }) if template_key == "assistant.general"));
        assert_eq!(request.model.unwrap().model, "step-5-preview");
        assert!(!request.clear_context_each_run);
        assert!(request.resource_selections.is_empty());
    }

    #[test]
    fn scheduling_cannot_submit_cli_or_frozen_authority_fields() {
        for (field, value) in [
            ("agent_type",json!("nomi")), ("agent_config",json!({})),
            ("prompt",json!("old alias")), ("agent_binding",json!({})),
            ("max_retries",json!(3)), ("backend",json!("acp")),
        ] {
            let mut body = create_body(); body[field] = value;
            assert!(serde_json::from_value::<CreateCronJobRequest>(body).is_err(), "{field}");
        }
    }

    #[test]
    fn specified_session_creation_does_not_require_shadow_agent_configuration() {
        let mut body = create_body();
        body.as_object_mut().unwrap().remove("agent_selection");
        body.as_object_mut().unwrap().remove("model");
        body["conversation_id"] = json!("0190f5fe-7c00-7a00-8000-000000000002");
        body["execution_mode"] = json!("existing");
        let request: CreateCronJobRequest = serde_json::from_value(body).unwrap();
        assert!(request.agent_selection.is_none()); assert!(request.model.is_none());
        assert!(request.conversation_id.is_some());
    }

    #[test]
    fn partial_updates_preserve_omitted_execution_configuration() {
        let request: UpdateCronJobRequest = serde_json::from_value(json!({"name":"Renamed"})).unwrap();
        assert_eq!(request.name.as_deref(), Some("Renamed"));
        assert!(request.agent_selection.is_none()); assert!(request.model.is_none());
        assert!(request.resource_selections.is_none()); assert!(request.clear_context_each_run.is_none());
        for field in ["agent_config","agent_type","max_retries","execution_mode"] {
            assert!(serde_json::from_value::<UpdateCronJobRequest>(json!({field:"removed"})).is_err());
        }
    }

    #[test]
    fn workbench_selection_rejects_malformed_personal_preset_ids() {
        for id in ["not-an-id", "preset_0190f5fe-7c00-7a00-8000-000000000001"] {
            let mut body=create_body();body["agent_selection"]=json!({"kind":"preset","preset_id":id});
            assert!(serde_json::from_value::<CreateCronJobRequest>(body).is_err());
        }
    }

    #[test]
    fn disabled_definition_keeps_authored_model_without_fabricating_a_binding() {
        let response=CronJobResponse {
            cron_job_id:JOB_ID.into(),name:"Weather".into(),description:None,enabled:false,
            schedule:CronScheduleDto::Every{every_ms:60000,description:None},message:"Weather".into(),
            execution_mode:"new_conversation".into(),
            metadata:CronJobMetadataDto {
                conversation_id:None,conversation_title:None,created_by:"user".into(),created_at:1,updated_at:2,
                agent_selection:None,model:Some(crate::AgentChatModelSelectionDto {
                    provider_id:"0190f5fe-7c00-7a00-8000-000000000001".into(),model:"step-5-preview".into()
                }),agent_binding:None,agent_label:None,clear_context_each_run:false,
            },
            state:CronJobStateDto{next_run_at_ms:None,last_run_at_ms:None,last_status:None,last_error:None,
                run_count:0,last_failure:None,configuration_error:Some("Select an Agent before enabling this task".into())},
        };
        let value=serde_json::to_value(&response).unwrap();
        assert_eq!(value["metadata"]["model"]["model"],"step-5-preview");
        assert!(value["metadata"].get("agent_binding").is_none());
        assert!(value["metadata"].get("agent_config").is_none());
        assert!(value["state"].get("retry_count").is_none());
        assert_eq!(serde_json::from_value::<CronJobResponse>(value).unwrap(),response);
    }

    // -- F. ListCronJobsQuery -------------------------------------------------

    #[test]
    fn list_query_with_conversation_id() {
        let raw = json!({"conversation_id": "0190f5fe-7c00-7a00-8abc-012345678901"});
        let q: ListCronJobsQuery = serde_json::from_value(raw).unwrap();
        assert_eq!(
            q.conversation_id.as_deref(),
            Some("0190f5fe-7c00-7a00-8abc-012345678901")
        );
    }

    #[test]
    fn list_query_empty() {
        let raw = json!({});
        let q: ListCronJobsQuery = serde_json::from_value(raw).unwrap();
        assert!(q.conversation_id.is_none());
    }

    #[test]
    fn list_query_rejects_numeric_and_malformed_conversation_ids() {
        assert!(
            serde_json::from_value::<ListCronJobsQuery>(json!({"conversation_id": 7})).is_err()
        );
        assert!(
            serde_json::from_value::<ListCronJobsQuery>(json!({"conversation_id": "conv_7"}))
                .is_err()
        );
    }

    // -- G. RunNowResponse / HasSkillResponse / SaveCronSkillRequest ----------

    #[test]
    fn run_now_response_serialize() {
        let r = RunNowResponse {
            conversation_id: "0190f5fe-7c00-7a00-8abc-012345678901".into(),
        };
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(
            json["conversation_id"],
            "0190f5fe-7c00-7a00-8abc-012345678901"
        );
    }

    #[test]
    fn run_now_response_roundtrip() {
        let r = RunNowResponse {
            conversation_id: "0190f5fe-7c00-7a00-8abc-012345678901".into(),
        };
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
            cron_job_id: JOB_ID.into(),
            status: "ok".into(),
            error: None,
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["cron_job_id"], JOB_ID);
        assert!(json.get("job_id").is_none());
        assert_eq!(json["status"], "ok");
        assert!(json.get("error").is_none());
    }

    #[test]
    fn executed_event_with_error() {
        let e = CronJobExecutedEvent {
            cron_job_id: JOB_ID_2.into(),
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
            cron_job_id: JOB_ID.into(),
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
            cron_job_id: JOB_ID.into(),
        };
        let json = serde_json::to_value(&p).unwrap();
        assert_eq!(json["cron_job_id"], JOB_ID);
        assert!(json.get("job_id").is_none());
    }

    #[test]
    fn removed_payload_roundtrip() {
        let p = CronJobRemovedPayload {
            cron_job_id: JOB_ID_2.into(),
        };
        let json = serde_json::to_string(&p).unwrap();
        let parsed: CronJobRemovedPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, p);
    }

    #[test]
    fn cron_event_payloads_reject_legacy_job_id_field() {
        assert!(
            serde_json::from_value::<CronJobExecutedEvent>(json!({
                "job_id": JOB_ID,
                "status": "ok"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<CronJobRemovedPayload>(json!({
                "job_id": JOB_ID
            }))
            .is_err()
        );
    }

    #[test]
    fn cron_job_run_response_rejects_legacy_id_fields() {
        let response = CronJobRunResponse {
            cron_job_run_id: JOB_ID_2.into(),
            cron_job_id: JOB_ID.into(),
            executed_at_ms: 1_700_000_000_000,
            status: "ok".into(),
            conversation_id:None, failure:None,
        };
        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["cron_job_run_id"], JOB_ID_2);
        assert_eq!(json["cron_job_id"], JOB_ID);
        assert!(json.get("id").is_none());
        assert!(json.get("job_id").is_none());

        assert!(
            serde_json::from_value::<CronJobRunResponse>(json!({
                "id": JOB_ID_2,
                "job_id": JOB_ID,
                "executed_at_ms": 1_700_000_000_000_i64,
                "status": "ok"
            }))
            .is_err()
        );
    }
}
