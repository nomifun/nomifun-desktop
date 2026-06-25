use nomifun_ai_agent::AgentSendError;
use nomifun_common::{AppError, ErrorChain, now_ms};
use nomifun_db::models::MessageRow;
use tracing::warn;

use crate::service::ConversationService;

impl ConversationService {
    pub(crate) async fn persist_send_failure_tip(&self, conversation_id: &str, err: &AppError) -> Option<MessageRow> {
        let Ok(conv_id) = conversation_id.parse::<i64>() else {
            warn!(
                conversation_id,
                "persist_send_failure_tip: non-numeric conversation id; skipping error tip persist"
            );
            return None;
        };
        let stream_error = AgentSendError::from_app_error_ref(err).into_stream_error();
        let row = MessageRow {
            id: Self::mint_msg_id(),
            conversation_id: conv_id,
            msg_id: None,
            r#type: "tips".into(),
            content: serde_json::json!({
                "content": &stream_error.message,
                "type": "error",
                "source": "send_failed",
                "code": err.error_code(),
                "details": err.error_details(),
                "error": stream_error,
            })
            .to_string(),
            position: Some("center".into()),
            status: Some("error".into()),
            hidden: false,
            created_at: now_ms(),
        };

        if let Err(store_err) = self.conversation_repo().insert_message(&row).await {
            warn!(
                conversation_id,
                error = %ErrorChain(&store_err),
                "Failed to persist send failure error tip"
            );
            return None;
        }

        Some(row)
    }
}
