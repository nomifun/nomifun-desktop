//! Completion notifier: implements `nomifun_requirement::CompletionNotifier` by
//! looking up the requirement's tag → bound webhook and sending a notification.
//!
//! Dependency direction: this crate depends on `nomifun-requirement` (for the
//! trait); `nomifun-requirement` does NOT depend on this crate. Mirrors how
//! `nomifun-idmm` implements `nomifun_requirement::IdmmHandle`.

use std::sync::Arc;

use async_trait::async_trait;
use nomifun_api_types::WebhookPlatform;
use nomifun_db::models::RequirementRow;
use nomifun_db::{ITagSettingRepository, IWebhookRepository};
use nomifun_requirement::CompletionNotifier;

use crate::sender::WebhookSender;
use crate::NotificationDeliveryStatus;

/// Truncate a content snippet for the notification card (keeps cards compact).
const MAX_CONTENT_CHARS: usize = 500;

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &s[..end]),
        None => s.to_string(),
    }
}

/// Human-readable completion status for the 【完成状态】 field.
fn status_label(status: &str) -> &'static str {
    match status {
        "done" => "已完成 (done)",
        "failed" => "失败 (failed)",
        "needs_review" => "待审核 (needs_review)",
        "cancelled" => "已取消 (cancelled)",
        _ => "完成 (completed)",
    }
}

/// Whether `status` is in the per-tag allowed event set.
pub fn event_allowed(status: &str, events: &[String]) -> bool {
    events.iter().any(|e| e == status)
}

pub struct CompletionNotifierImpl {
    tag_settings: Arc<dyn ITagSettingRepository>,
    webhooks: Arc<dyn IWebhookRepository>,
    sender: Arc<dyn WebhookSender>,
}

impl CompletionNotifierImpl {
    pub fn new(
        tag_settings: Arc<dyn ITagSettingRepository>,
        webhooks: Arc<dyn IWebhookRepository>,
        sender: Arc<dyn WebhookSender>,
    ) -> Self {
        Self {
            tag_settings,
            webhooks,
            sender,
        }
    }

    pub fn into_arc(self) -> Arc<dyn CompletionNotifier> {
        Arc::new(self)
    }

    /// Deliver one completion notification and return a stable external-action
    /// status for observability. Requirement completion still treats delivery
    /// as best effort; the trait adapter below logs failures and never mutates
    /// the completed Requirement fact.
    pub async fn notify_completion_with_status(
        &self,
        requirement: &RequirementRow,
    ) -> NotificationDeliveryStatus {
        let setting = match self.tag_settings.get(&requirement.tag).await {
            Ok(Some(setting)) => setting,
            Ok(None) => return NotificationDeliveryStatus::SkippedUnbound,
            Err(_) => {
                return NotificationDeliveryStatus::Failed {
                    webhook_id: None,
                    code: "NOTIFICATION_CONFIGURATION_UNAVAILABLE",
                };
            }
        };
        let events = setting
            .notify_events
            .split(',')
            .filter(|event| !event.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let Some(webhook_id) = setting.webhook_id else {
            return NotificationDeliveryStatus::SkippedUnbound;
        };
        let webhook = match self.webhooks.get_by_webhook_id(&webhook_id).await {
            Ok(Some(webhook)) => webhook,
            Ok(None) | Err(_) => {
                return NotificationDeliveryStatus::Failed {
                    webhook_id: Some(webhook_id),
                    code: "NOTIFICATION_RESOURCE_UNAVAILABLE",
                };
            }
        };
        if !webhook.enabled {
            return NotificationDeliveryStatus::SkippedDisabled {
                webhook_id: webhook.webhook_id,
            };
        }
        if !event_allowed(&requirement.status, &events) {
            return NotificationDeliveryStatus::SkippedFiltered {
                webhook_id: webhook.webhook_id,
                event: requirement.status.clone(),
            };
        }

        let fields = completion_fields(requirement);
        let title = format!(
            "需求{}: {}",
            status_label(&requirement.status),
            requirement.title
        );
        match self
            .sender
            .send_card(
                WebhookPlatform::from_db(&webhook.platform),
                &webhook.url,
                webhook.secret.as_deref(),
                &title,
                &fields,
            )
            .await
        {
            Ok(()) => NotificationDeliveryStatus::Delivered {
                webhook_id: webhook.webhook_id,
            },
            Err(error) if error.outcome_unknown() => {
                NotificationDeliveryStatus::OutcomeUnknown {
                    webhook_id: webhook.webhook_id,
                    code: "NOTIFICATION_DELIVERY_OUTCOME_UNKNOWN",
                    recovery: "inspect the destination before retrying; never retry automatically",
                }
            }
            Err(_) => NotificationDeliveryStatus::Failed {
                webhook_id: Some(webhook.webhook_id),
                code: "NOTIFICATION_DELIVERY_FAILED",
            },
        }
    }
}

#[async_trait]
impl CompletionNotifier for CompletionNotifierImpl {
    async fn notify_completion(&self, requirement: &RequirementRow) {
        let status = self.notify_completion_with_status(requirement).await;
        match status {
            NotificationDeliveryStatus::Failed { webhook_id, code } => {
                tracing::warn!(
                    webhook_id = webhook_id.as_deref().unwrap_or("unresolved"),
                    requirement_id = %requirement.requirement_id,
                    code,
                    "completion webhook delivery failed"
                );
            }
            NotificationDeliveryStatus::OutcomeUnknown {
                webhook_id,
                code,
                recovery,
            } => {
                tracing::warn!(
                    webhook_id,
                    requirement_id = %requirement.requirement_id,
                    code,
                    recovery,
                    "completion webhook delivery outcome is unknown"
                );
            }
            _ => {}
        }
    }
}

fn completion_fields(requirement: &RequirementRow) -> Vec<(String, String)> {
    // Template: 【需求id】【需求名】【需求内容】【完成状态】【完成记录(报告)】
    vec![
        ("需求id".to_string(), requirement.requirement_id.clone()),
        ("需求名".to_string(), requirement.title.clone()),
        (
            "需求内容".to_string(),
            truncate(&requirement.content, MAX_CONTENT_CHARS),
        ),
        (
            "完成状态".to_string(),
            status_label(&requirement.status).to_string(),
        ),
        (
            "完成记录(报告)".to_string(),
            requirement
                .completion_note
                .as_deref()
                .map(|note| truncate(note, MAX_CONTENT_CHARS))
                .unwrap_or_else(|| "-".to_string()),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::event_allowed;
    #[test]
    fn allows_when_status_in_set() {
        assert!(event_allowed("done", &["done".to_string(), "failed".to_string()]));
        assert!(!event_allowed("needs_review", &["done".to_string(), "failed".to_string()]));
    }
    #[test]
    fn empty_set_allows_nothing() {
        assert!(!event_allowed("done", &[]));
    }
}
