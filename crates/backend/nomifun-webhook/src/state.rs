use crate::service::WebhookService;

/// Router state for the webhook + tag-settings endpoints.
#[derive(Clone)]
pub struct WebhookRouterState {
    pub service: WebhookService,
}
