//! Router state for the browser-secret endpoints (P3-X2).

use crate::service::SecretService;

/// Router state for `/api/browser-secrets/*`.
#[derive(Clone)]
pub struct SecretRouterState {
    pub service: SecretService,
}

impl SecretRouterState {
    pub fn new(service: SecretService) -> Self {
        Self { service }
    }
}
