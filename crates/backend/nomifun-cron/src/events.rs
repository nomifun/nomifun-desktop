use std::sync::Arc;

use nomifun_api_types::{CronJobExecutedEvent, CronJobRemovedPayload, CronJobResponse, WebSocketMessage};
use nomifun_conversation::ConversationService;
use nomifun_realtime::EventBroadcaster;
use serde_json::json;
use tracing::error;

#[derive(Clone)]
pub struct CronEventEmitter {
    broadcaster: Arc<dyn EventBroadcaster>,
}

impl CronEventEmitter {
    pub fn new(broadcaster: Arc<dyn EventBroadcaster>) -> Self {
        Self { broadcaster }
    }

    pub fn emit_job_created(&self, job: &CronJobResponse) {
        self.broadcast("cron.job-created", job);
    }

    pub fn emit_job_updated(&self, job: &CronJobResponse) {
        self.broadcast("cron.job-updated", job);
    }

    pub fn emit_job_removed(&self, job_id: &str) {
        self.broadcast(
            "cron.job-removed",
            &CronJobRemovedPayload {
                job_id: job_id.to_owned(),
            },
        );
    }

    pub fn emit_job_executed(&self, job_id: &str, status: &str, err: Option<&str>) {
        self.broadcast(
            "cron.job-executed",
            &CronJobExecutedEvent {
                job_id: job_id.to_owned(),
                status: status.to_owned(),
                error: err.map(|s| s.to_owned()),
            },
        );
    }

    pub fn emit_conversation_tips(&self, conversation_id: &str, content: &str, tip_type: &str) {
        let payload = json!({
            "conversation_id": conversation_id,
            "msg_id": ConversationService::mint_msg_id(),
            "type": "tips",
            "data": {
                "content": content,
                "type": tip_type,
            },
            "hidden": false,
        });
        self.broadcaster
            .broadcast(WebSocketMessage::new("message.stream", payload));
    }

    fn broadcast<T: serde::Serialize>(&self, event_name: &str, payload: &T) {
        let value = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                error!(event_name, error = %e, "Failed to serialize event payload");
                return;
            }
        };
        self.broadcaster.broadcast(WebSocketMessage::new(event_name, value));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nomifun_api_types::{
        CronJobExecutedEvent, CronJobMetadataDto, CronJobPayloadDto, CronJobRemovedPayload, CronJobResponse,
        CronJobStateDto, CronJobTargetDto, CronScheduleDto,
    };

    struct RecordingBroadcaster {
        events: std::sync::Mutex<Vec<WebSocketMessage<serde_json::Value>>>,
    }

    impl RecordingBroadcaster {
        fn new() -> Self {
            Self {
                events: std::sync::Mutex::new(vec![]),
            }
        }

        fn events(&self) -> Vec<WebSocketMessage<serde_json::Value>> {
            self.events.lock().unwrap().clone()
        }
    }

    impl EventBroadcaster for RecordingBroadcaster {
        fn broadcast(&self, event: WebSocketMessage<serde_json::Value>) {
            self.events.lock().unwrap().push(event);
        }
    }

    fn make_emitter() -> (CronEventEmitter, Arc<RecordingBroadcaster>) {
        let bc = Arc::new(RecordingBroadcaster::new());
        let emitter = CronEventEmitter::new(bc.clone());
        (emitter, bc)
    }

    fn sample_response() -> CronJobResponse {
        CronJobResponse {
            id: "cron_123".into(),
            name: "Test Job".into(),
            description: Some("Test description".into()),
            enabled: true,
            schedule: CronScheduleDto::Every {
                every_ms: 60000,
                description: Some("every minute".into()),
            },
            target: CronJobTargetDto {
                payload: CronJobPayloadDto::Message { text: "hello".into() },
                execution_mode: Some("existing".into()),
                target_kind: "agent".into(),
            },
            metadata: CronJobMetadataDto {
                conversation_id: 1,
                conversation_title: None,
                agent_type: "acp".into(),
                created_by: "user".into(),
                created_at: 1000,
                updated_at: 2000,
                agent_config: None,
            },
            state: CronJobStateDto {
                next_run_at_ms: Some(61000),
                last_run_at_ms: None,
                last_status: None,
                last_error: None,
                run_count: 0,
                retry_count: 0,
                max_retries: 3,
            },
        }
    }

    #[test]
    fn job_created_event_shape() {
        let (emitter, bc) = make_emitter();
        let resp = sample_response();
        emitter.emit_job_created(&resp);

        let events = bc.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "cron.job-created");

        let parsed: CronJobResponse = serde_json::from_value(events[0].data.clone()).unwrap();
        assert_eq!(parsed.id, "cron_123");
        assert_eq!(parsed.name, "Test Job");
    }

    #[test]
    fn job_updated_event_shape() {
        let (emitter, bc) = make_emitter();
        let resp = sample_response();
        emitter.emit_job_updated(&resp);

        let events = bc.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "cron.job-updated");

        let parsed: CronJobResponse = serde_json::from_value(events[0].data.clone()).unwrap();
        assert_eq!(parsed.id, "cron_123");
    }

    #[test]
    fn job_removed_event_shape() {
        let (emitter, bc) = make_emitter();
        emitter.emit_job_removed("cron_456");

        let events = bc.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "cron.job-removed");

        let parsed: CronJobRemovedPayload = serde_json::from_value(events[0].data.clone()).unwrap();
        assert_eq!(parsed.job_id, "cron_456");
    }

    #[test]
    fn job_executed_success_event() {
        let (emitter, bc) = make_emitter();
        emitter.emit_job_executed("cron_789", "ok", None);

        let events = bc.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, "cron.job-executed");

        let parsed: CronJobExecutedEvent = serde_json::from_value(events[0].data.clone()).unwrap();
        assert_eq!(parsed.job_id, "cron_789");
        assert_eq!(parsed.status, "ok");
        assert!(parsed.error.is_none());
    }

    #[test]
    fn job_executed_error_event() {
        let (emitter, bc) = make_emitter();
        emitter.emit_job_executed("cron_789", "error", Some("timeout"));

        let events = bc.events();
        assert_eq!(events.len(), 1);

        let parsed: CronJobExecutedEvent = serde_json::from_value(events[0].data.clone()).unwrap();
        assert_eq!(parsed.status, "error");
        assert_eq!(parsed.error.as_deref(), Some("timeout"));
    }

    #[test]
    fn job_executed_skipped_event() {
        let (emitter, bc) = make_emitter();
        emitter.emit_job_executed("cron_789", "skipped", None);

        let events = bc.events();
        let parsed: CronJobExecutedEvent = serde_json::from_value(events[0].data.clone()).unwrap();
        assert_eq!(parsed.status, "skipped");
    }

    #[test]
    fn multiple_events_accumulate() {
        let (emitter, bc) = make_emitter();
        let resp = sample_response();
        emitter.emit_job_created(&resp);
        emitter.emit_job_updated(&resp);
        emitter.emit_job_removed("cron_123");
        emitter.emit_job_executed("cron_123", "ok", None);

        let events = bc.events();
        assert_eq!(events.len(), 4);
        assert_eq!(events[0].name, "cron.job-created");
        assert_eq!(events[1].name, "cron.job-updated");
        assert_eq!(events[2].name, "cron.job-removed");
        assert_eq!(events[3].name, "cron.job-executed");
    }
}
