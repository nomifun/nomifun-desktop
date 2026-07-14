use nomifun_api_types::{ConversationArtifactResponse, WebSocketMessage};
use nomifun_db::ConversationArtifactRow;
use nomifun_realtime::UserEventSink;
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::error::CronError;
use crate::types::CronJob;

/// Parse a string-keyed conversation id into the integer DB key. Cron carries
/// conversation ids as `String` through the agent path (Option A); artifact
/// rows are keyed by `i64`, so we convert at this boundary. An unparseable id
/// degrades to `0`, which matches no conversation row (the upsert/broadcast is
/// then a harmless no-op rather than a panic).
fn parse_conversation_id(conversation_id: &str) -> i64 {
    conversation_id.parse::<i64>().unwrap_or(0)
}

pub(crate) fn build_cron_trigger_artifact(
    conversation_id: &str,
    job: &CronJob,
    created_at: i64,
) -> ConversationArtifactRow {
    let payload = json!({
        "cron_job_id": job.id,
        "cron_job_name": job.name,
        "triggered_at": created_at,
    });

    ConversationArtifactRow {
        // `id` is assigned by SQLite on insert; `upsert_artifact` ignores this
        // placeholder. `cron_trigger` rows are always fresh inserts (one per
        // trigger), no longer deduplicated by a composite string id.
        id: 0,
        conversation_id: parse_conversation_id(conversation_id),
        cron_job_id: Some(job.id.clone()),
        kind: "cron_trigger".into(),
        status: "active".into(),
        payload: payload.to_string(),
        created_at,
        updated_at: created_at,
    }
}

pub(crate) fn build_skill_suggest_artifact(
    conversation_id: &str,
    job_id: &str,
    name: &str,
    description: &str,
    skill_content: &str,
    now: i64,
) -> ConversationArtifactRow {
    let payload = json!({
        "cron_job_id": job_id,
        "name": name,
        "description": description,
        "skillContent": skill_content,
    });

    ConversationArtifactRow {
        // `id` is assigned by SQLite; idempotency for `skill_suggest` is the
        // partial-unique `(conversation_id, cron_job_id)` constraint that
        // `upsert_artifact` targets, not this placeholder.
        id: 0,
        conversation_id: parse_conversation_id(conversation_id),
        cron_job_id: Some(job_id.to_owned()),
        kind: "skill_suggest".into(),
        status: "pending".into(),
        payload: payload.to_string(),
        created_at: now,
        updated_at: now,
    }
}

pub(crate) fn artifact_response_from_row(
    row: &ConversationArtifactRow,
) -> Result<ConversationArtifactResponse, CronError> {
    Ok(ConversationArtifactResponse {
        id: row.id,
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
            id: "cron_1".into(),
            user_id: "user_1".into(),
            name: "Daily Report".into(),
            enabled: true,
            schedule: CronSchedule::Every {
                every_ms: 60_000,
                description: None,
            },
            message: "Run".into(),
            execution_mode: ExecutionMode::NewConversation,
            agent_config: None,
            conversation_id: "conv_1".into(),
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
            "conv_1",
            "cron_1",
            "daily-report",
            "Daily report",
            "---\nname: daily-report\n---\nUse it.",
            1234,
        );

        let response = artifact_response_from_row(&row).unwrap();
        assert_eq!(response.kind, nomifun_api_types::ConversationArtifactKind::SkillSuggest);
        assert_eq!(response.status, nomifun_api_types::ConversationArtifactStatus::Pending);
        assert_eq!(response.payload["name"], "daily-report");
    }

    #[test]
    fn private_artifact_events_are_scoped_to_each_conversation_owner() {
        let user_events = RecordingUserEvents::new();
        let owner_a = build_cron_trigger_artifact("1", &sample_job(), 1000);
        let owner_b = build_cron_trigger_artifact("2", &sample_job(), 2000);

        emit_artifact(&user_events, "owner-a", &owner_a).unwrap();
        emit_artifact(&user_events, "owner-b", &owner_b).unwrap();

        let deliveries = user_events.deliveries.lock().unwrap();
        assert_eq!(deliveries.len(), 2);
        assert_eq!(deliveries[0].0, "owner-a");
        assert_eq!(deliveries[0].1.name, "conversation.artifact");
        assert_eq!(deliveries[0].1.data["conversation_id"], 1);
        assert_eq!(deliveries[1].0, "owner-b");
        assert_eq!(deliveries[1].1.name, "conversation.artifact");
        assert_eq!(deliveries[1].1.data["conversation_id"], 2);
    }

    #[test]
    fn builds_cron_trigger_payload() {
        let row = build_cron_trigger_artifact("conv_1", &sample_job(), 1234);
        let response = artifact_response_from_row(&row).unwrap();
        assert_eq!(response.kind, nomifun_api_types::ConversationArtifactKind::CronTrigger);
        assert_eq!(response.payload["cron_job_id"], "cron_1");
        assert_eq!(response.payload["cron_job_name"], "Daily Report");
    }
}
