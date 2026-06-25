use std::sync::Arc;

use crate::service::AssetService;

/// Shared state for the public asset router.
#[derive(Clone)]
pub struct AssetRouterState {
    pub service: Arc<AssetService>,
}

impl Default for AssetRouterState {
    fn default() -> Self {
        Self {
            service: Arc::new(AssetService),
        }
    }
}
