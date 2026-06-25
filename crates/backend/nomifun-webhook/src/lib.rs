//! Webhook management + AutoWork completion notifications.
//!
//! - CRUD over reusable outbound webhooks (v1: Lark/飞书 custom bots) and per-tag
//!   settings (bound webhook + description) layered over the implicit tags.
//! - `CompletionNotifierImpl` implements `nomifun_requirement::CompletionNotifier`
//!   so a requirement reaching a terminal state notifies its tag's bound webhook.

pub mod error;
pub mod notifier;
pub mod routes;
pub mod sender;
pub mod service;
pub mod state;

pub use error::WebhookError;
pub use notifier::CompletionNotifierImpl;
pub use routes::webhook_routes;
pub use sender::{DefaultWebhookSender, WebhookSender};
pub use service::WebhookService;
pub use state::WebhookRouterState;
