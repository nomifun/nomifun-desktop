use nomifun_ai_agent::AgentSendError;
use nomifun_common::{AppError, ErrorChain, now_ms};
use nomifun_db::models::MessageRow;
use nomifun_db::MessageRowUpdate;
use tracing::warn;

use crate::service::ConversationService;

/// 构造"图片已移除"提示行的 content JSON 串(与 persist 分离便于测试)。
pub(crate) fn images_stripped_tip_content() -> String {
    serde_json::json!({
        "content": "当前模型不支持图片输入，已自动移除图片并重试。",
        "type": "warning",
        "source": "images_stripped",
    })
    .to_string()
}

impl ConversationService {
    pub(crate) async fn persist_send_failure_tip(
        &self,
        conversation_id: &str,
        turn_id: Option<&str>,
        err: &AppError,
    ) -> Option<MessageRow> {
        let conv_id = conversation_id.to_owned();
        let stream_error = AgentSendError::from_app_error_ref(err).into_stream_error();
        let id = Self::mint_msg_id();
        let row = MessageRow {
            id: 0,
            message_id: id.clone(),
            conversation_id: conv_id,
            msg_id: Some(id),
            r#type: "tips".into(),
            content: serde_json::json!({
                "content": &stream_error.message,
                "type": "error",
                "source": "send_failed",
                "code": err.error_code(),
                "details": err.error_details(),
                "error": stream_error,
                "turn_id": turn_id,
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

    /// 在会话里插入一条"图片已移除"警告提示(tips)。仅供用户查看,不回传模型。
    pub(crate) async fn persist_images_stripped_tip(&self, conversation_id: &str) -> Option<MessageRow> {
        let conv_id = conversation_id.to_owned();
        let message_id = Self::mint_msg_id();
        let row = MessageRow {
            id: 0,
            message_id,
            conversation_id: conv_id,
            msg_id: None,
            r#type: "tips".into(),
            content: images_stripped_tip_content(),
            position: Some("center".into()),
            status: None,
            hidden: false,
            created_at: now_ms(),
        };
        if let Err(store_err) = self.conversation_repo().insert_message(&row).await {
            warn!(
                conversation_id,
                error = %ErrorChain(&store_err),
                "Failed to persist images-stripped tip"
            );
            return None;
        }
        Some(row)
    }

    /// Persist a non-terminal warning when failover cannot yet prove that the
    /// old runtime exited. The Conversation deliberately remains Running: this
    /// row explains the retained safety fence without pretending the turn is
    /// idle or authorizing an overlapping replacement.
    pub(crate) async fn persist_model_failover_teardown_tip(
        &self,
        conversation_id: &str,
        turn_id: Option<&str>,
    ) -> Option<MessageRow> {
        let message_id = Self::mint_msg_id();
        let row = MessageRow {
            id: 0,
            message_id: message_id.clone(),
            conversation_id: conversation_id.to_owned(),
            msg_id: Some(message_id),
            r#type: "tips".into(),
            content: serde_json::json!({
                "content": "模型故障转移正在安全清理旧运行时并同步状态。为避免重复执行，暂时不会启动新的运行时；请先等待，也可以使用“停止”请求继续清理。若长时间未恢复，请重启服务后重试。",
                "type": "warning",
                "source": "model_failover_teardown",
                "retryable": true,
                "turn_id": turn_id,
            })
            .to_string(),
            position: Some("center".into()),
            status: Some("work".into()),
            hidden: false,
            created_at: now_ms(),
        };

        if let Err(store_err) = self.conversation_repo().insert_message(&row).await {
            warn!(
                conversation_id,
                error = %ErrorChain(&store_err),
                "Failed to persist model-failover teardown warning"
            );
            return None;
        }
        Some(row)
    }

    /// Resolve the exact warning row in place once old-runtime exit is proven.
    /// Keeping the same message identity lets every mounted client replace the
    /// transient "waiting" state instead of retaining a stale work item in
    /// conversation history.
    pub(crate) async fn resolve_model_failover_teardown_tip(
        &self,
        row: &mut MessageRow,
        turn_id: Option<&str>,
    ) -> bool {
        let content = serde_json::json!({
            "content": "旧运行时已安全退出，正在同步最新配置。",
            "type": "info",
            "source": "model_failover_teardown",
            "state": "resolved",
            "retryable": false,
            "turn_id": turn_id,
        })
        .to_string();
        let update = MessageRowUpdate {
            content: Some(content.clone()),
            status: Some(Some("finish".into())),
            hidden: None,
        };
        if let Err(store_err) = self
            .conversation_repo()
            .update_message(&row.message_id, &update)
            .await
        {
            warn!(
                conversation_id = %row.conversation_id,
                message_id = %row.message_id,
                error = %ErrorChain(&store_err),
                "Failed to resolve model-failover teardown warning"
            );
            return false;
        }
        row.content = content;
        row.status = Some("finish".into());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_stripped_tip_has_warning_type_and_source() {
        let s = images_stripped_tip_content();
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["type"], "warning");
        assert_eq!(v["source"], "images_stripped");
        assert!(v["content"].as_str().unwrap().contains("图片"));
    }
}
