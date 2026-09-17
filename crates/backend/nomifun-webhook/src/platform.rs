//! Typed platform-notification contract.
//!
//! Notification delivery is driven by an Automation/domain event and its
//! configured binding. It is not an Agent Capability grant. The empty action
//! inventory below is intentional: if a future product permits an Agent to send
//! a notification, it must add a separately reviewed `notification.send`
//! action instead of exposing this consumer or its transport credentials.

use nomifun_api_types::WebhookPlatform;
use serde::Serialize;

pub const NOTIFICATION_PLATFORM_SERVICE_ID: &str = "notification.configuration";
pub const NOTIFICATION_AGENT_ACTION_IDS: [&str; 0] = [];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum NotificationBindingState {
    Unbound {
        tag: String,
    },
    Bound {
        tag: String,
        webhook_id: String,
        platform: WebhookPlatform,
        enabled: bool,
        notify_events: Vec<String>,
    },
    MissingResource {
        tag: String,
        webhook_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum NotificationDeliveryStatus {
    Delivered {
        webhook_id: String,
    },
    SkippedUnbound,
    SkippedDisabled {
        webhook_id: String,
    },
    SkippedFiltered {
        webhook_id: String,
        event: String,
    },
    Failed {
        webhook_id: Option<String>,
        code: &'static str,
    },
    OutcomeUnknown {
        webhook_id: String,
        code: &'static str,
        recovery: &'static str,
    },
}

impl NotificationDeliveryStatus {
    pub fn is_terminal_success(&self) -> bool {
        matches!(
            self,
            Self::Delivered { .. }
                | Self::SkippedUnbound
                | Self::SkippedDisabled { .. }
                | Self::SkippedFiltered { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_notification_never_projects_agent_actions() {
        assert!(NOTIFICATION_AGENT_ACTION_IDS.is_empty());
        assert_eq!(
            NOTIFICATION_PLATFORM_SERVICE_ID,
            "notification.configuration"
        );
    }

    #[test]
    fn delivery_status_distinguishes_skips_failures_and_success() {
        assert!(NotificationDeliveryStatus::SkippedUnbound.is_terminal_success());
        assert!(
            NotificationDeliveryStatus::Delivered {
                webhook_id: "hook-a".into()
            }
            .is_terminal_success()
        );
        assert!(
            !NotificationDeliveryStatus::Failed {
                webhook_id: Some("hook-a".into()),
                code: "NOTIFICATION_DELIVERY_FAILED",
            }
            .is_terminal_success()
        );
        assert!(
            !NotificationDeliveryStatus::OutcomeUnknown {
                webhook_id: "hook-a".into(),
                code: "NOTIFICATION_DELIVERY_OUTCOME_UNKNOWN",
                recovery: "do not retry automatically",
            }
            .is_terminal_success()
        );
    }
}
