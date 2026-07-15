use nomifun_api_types::{ConversationArtifactResponse, WebSocketMessage};
use nomifun_common::{ConversationArtifactId, ConversationId, CronJobId, UserId};
use nomifun_db::ConversationArtifactRow;
use nomifun_realtime::UserEventSink;
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::error::CronError;
use crate::types::CronJob;

pub(crate) fn build_cron_trigger_artifact(
    conversation_id: &str,
    job: &CronJob,
    created_at: i64,
) -> Result<ConversationArtifactRow, CronError> {
    ConversationId::try_from(conversation_id).map_err(|error| {
        CronError::Scheduler(format!("invalid artifact conversation id: {error}"))
    })?;
    CronJobId::try_from(job.id.as_str())
        .map_err(|error| CronError::Scheduler(format!("invalid artifact cron job id: {error}")))?;
    let payload = json!({
        "cron_job_id": job.id,
        "cron_job_name": job.name,
        "triggered_at": created_at,
    });

    Ok(ConversationArtifactRow {
        id: ConversationArtifactId::new().into_string(),
        conversation_id: conversation_id.to_owned(),
        cron_job_id: Some(job.id.clone()),
        kind: "cron_trigger".into(),
        status: "active".into(),
        payload: payload.to_string(),
        created_at,
        updated_at: created_at,
    })
}

pub(crate) fn build_skill_suggest_artifact(
    conversation_id: &str,
    job_id: &str,
    name: &str,
    description: &str,
    skill_content: &str,
    now: i64,
) -> Result<ConversationArtifactRow, CronError> {
    ConversationId::try_from(conversation_id).map_err(|error| {
        CronError::Scheduler(format!("invalid artifact conversation id: {error}"))
    })?;
    CronJobId::try_from(job_id)
        .map_err(|error| CronError::Scheduler(format!("invalid artifact cron job id: {error}")))?;
    let payload = json!({
        "cron_job_id": job_id,
        "name": name,
        "description": description,
        "skillContent": skill_content,
    });

    Ok(ConversationArtifactRow {
        id: ConversationArtifactId::new().into_string(),
        conversation_id: conversation_id.to_owned(),
        cron_job_id: Some(job_id.to_owned()),
        kind: "skill_suggest".into(),
        status: "pending".into(),
        payload: payload.to_string(),
        created_at: now,
        updated_at: now,
    })
}

pub(crate) fn artifact_response_from_row(
    row: &ConversationArtifactRow,
) -> Result<ConversationArtifactResponse, CronError> {
    ConversationArtifactId::try_from(row.id.as_str())
        .map_err(|error| CronError::Scheduler(format!("invalid artifact id: {error}")))?;
    ConversationId::try_from(row.conversation_id.as_str())
        .map_err(|error| CronError::Scheduler(format!("invalid artifact conversation id: {error}")))?;
    if let Some(job_id) = row.cron_job_id.as_deref() {
        CronJobId::try_from(job_id)
            .map_err(|error| CronError::Scheduler(format!("invalid artifact cron job id: {error}")))?;
    }
    Ok(ConversationArtifactResponse {
        id: row.id.clone(),
        conversation_id: row.conversation_id.clone(),
        cron_job_id: row.cron_job_id.clone(),
        kind: parse_enum(&row.kind)?,
        status: parse_enum(&row.status)?,
        payload: serde_json::from_str(&row.payload)
            .map_err(|e| CronError::Scheduler(format!("invalid artifact payload JSON: {e}")))?,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

pub(crate) fn emit_artifact(
    user_events: &dyn UserEventSink,
    owner_id: &str,
    row: &ConversationArtifactRow,
) -> Result<(), CronError> {
    UserId::try_from(owner_id)
        .map_err(|error| CronError::Scheduler(format!("invalid artifact owner id: {error}")))?;
    let payload = serde_json::to_value(artifact_response_from_row(row)?)
        .map_err(|e| CronError::Scheduler(format!("failed to serialize artifact event: {e}")))?;
    user_events.send_to_user(
        owner_id,
        WebSocketMessage::new("conversation.artifact", payload),
    );
    Ok(())
}

fn parse_enum<T: DeserializeOwned>(value: &str) -> Result<T, CronError> {
    serde_json::from_value(serde_json::Value::String(value.to_owned()))
        .map_err(|e| CronError::Scheduler(format!("invalid artifact enum value '{value}': {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CreatedBy, CronJob, CronSchedule, ExecutionMode};
    use std::sync::Mutex;

    const JOB_ID: &str = "cron_0190f5fe-7c00-7a00-8000-000000000001";
    const USER_ID: &str = "user_0190f5fe-7c00-7a00-8000-000000000001";
    const USER_ID_2: &str = "user_0190f5fe-7c00-7a00-8000-000000000002";
    const CONVERSATION_ID: &str = "conv_0190f5fe-7c00-7a00-8000-000000000001";
    const CONVERSATION_ID_2: &str = "conv_0190f5fe-7c00-7a00-8000-000000000002";

    struct RecordingUserEvents {
        deliveries: Mutex<Vec<(String, WebSocketMessage<serde_json::Value>)>>,
    }

    impl RecordingUserEvents {
        fn new() -> Self {
            Self {
                deliveries: Mutex::new(Vec::new()),
            }
        }
    }

    impl UserEventSink for RecordingUserEvents {
        fn send_to_user(&self, user_id: &str, event: WebSocketMessage<serde_json::Value>) {
            self.deliveries
                .lock()
                .unwrap()
                .push((user_id.to_owned(), event));
        }
    }

    fn sample_job() -> CronJob {
        CronJob {
            id: JOB_ID.into(),
            user_id: USER_ID.into(),
            name: "Daily Report".into(),
            enabled: true,
            schedule: CronSchedule::Every {
                every_ms: 60_000,
                description: None,
            },
            message: "Run".into(),
            execution_mode: ExecutionMode::NewConversation,
            agent_config: None,
            conversation_id: Some(CONVERSATION_ID.into()),
            conversation_title: None,
            agent_type: "acp".into(),
            created_by: CreatedBy::User,
            skill_content: None,
            description: None,
            created_at: 1000,
            updated_at: 1000,
            next_run_at: Some(2000),
            last_run_at: None,
            last_status: None,
            last_error: None,
            run_count: 0,
            retry_count: 0,
            max_retries: 3,
        }
    }

    #[test]
    fn builds_skill_suggest_response() {
        let row = build_skill_suggest_artifact(
            CONVERSATION_ID,
            JOB_ID,
            "daily-report",
            "Daily report",
            "---\nname: daily-report\n---\nUse it.",
            1234,
        )
        .unwrap();

        let response = artifact_response_from_row(&row).unwrap();
        assert_eq!(response.kind, nomifun_api_types::ConversationArtifactKind::SkillSuggest);
        assert_eq!(response.status, nomifun_api_types::ConversationArtifactStatus::Pending);
        assert_eq!(response.payload["name"], "daily-report");
    }

    #[test]
    fn private_artifact_events_are_scoped_to_each_conversation_owner() {
        let user_events = RecordingUserEvents::new();
        let owner_a_id = CONVERSATION_ID;
        let owner_b_id = CONVERSATION_ID_2;
        let owner_a = build_cron_trigger_artifact(owner_a_id, &sample_job(), 1000).unwrap();
        let owner_b = build_cron_trigger_artifact(owner_b_id, &sample_job(), 2000).unwrap();

        emit_artifact(&user_events, USER_ID, &owner_a).unwrap();
        emit_artifact(&user_events, USER_ID_2, &owner_b).unwrap();

        let deliveries = user_events.deliveries.lock().unwrap();
        assert_eq!(deliveries.len(), 2);
        assert_eq!(deliveries[0].0, USER_ID);
        assert_eq!(deliveries[0].1.name, "conversation.artifact");
        assert_eq!(deliveries[0].1.data["conversation_id"], owner_a_id);
        assert_eq!(deliveries[1].0, USER_ID_2);
        assert_eq!(deliveries[1].1.name, "conversation.artifact");
        assert_eq!(deliveries[1].1.data["conversation_id"], owner_b_id);
    }

    #[test]
    fn builds_cron_trigger_payload() {
        let row = build_cron_trigger_artifact(CONVERSATION_ID, &sample_job(), 1234).unwrap();
        let response = artifact_response_from_row(&row).unwrap();
        assert_eq!(response.kind, nomifun_api_types::ConversationArtifactKind::CronTrigger);
        assert_eq!(response.payload["cron_job_id"], JOB_ID);
        assert_eq!(response.payload["cron_job_name"], "Daily Report");
    }
}
