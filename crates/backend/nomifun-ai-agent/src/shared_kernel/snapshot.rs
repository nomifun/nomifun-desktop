use std::collections::HashMap;

use agent_client_protocol::schema::UsageUpdate;

use super::{ConfigKey, ConfigValue, ModeId, ModelId};

/// Decoded per-session runtime state loaded from `acp_session.session_config.runtime`.
///
/// Only carries the user's last *choices* — the enumerations of what
/// the agent supports (mode list, model list, config schema) come from
/// the CLI's session response after initialization.
///
/// Shared between the factory (seeds `AcpSessionParams`), the aggregate
/// root (`AcpSession::preload_persisted`), and the persistence consumer
/// (`AcpSessionSyncService::load_snapshot_state`), so it lives in
/// `shared_kernel` rather than any of those layers.
#[derive(Debug, Clone, Default)]
pub struct PersistedSessionState {
    pub current_mode_id: Option<ModeId>,
    pub current_model_id: Option<ModelId>,
    pub config_selections: HashMap<ConfigKey, ConfigValue>,
    pub context_usage: Option<UsageUpdate>,
}
