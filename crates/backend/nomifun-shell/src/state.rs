use std::sync::Arc;

use nomifun_model_invoke::ModelInvokeService;
use nomifun_system::{ClientPrefService, ProviderService};

use crate::shell::ShellService;
use crate::stt::SttService;

#[derive(Clone)]
pub struct ShellRouterState {
    pub shell_service: Arc<ShellService>,
    pub stt_service: Arc<SttService>,
    pub client_pref_service: ClientPrefService,
    pub provider_service: Option<ProviderService>,
    /// Unified invoke layer for `/api/tts` (speech synthesis). `None` mirrors
    /// `provider_service`: unit tests without a catalog leave it unwired and
    /// the route degrades to a config error.
    pub model_invoke_service: Option<Arc<ModelInvokeService>>,
}
