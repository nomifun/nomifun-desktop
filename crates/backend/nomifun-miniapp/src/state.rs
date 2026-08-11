//! Router state for both mini-app routers.
//!
//! One state, shared by the authenticated management router and the auth-exempt
//! serve router: the document the serve route hands out must be the one the last
//! solidify wrote, and a second service over a second repository handle would
//! make that a coincidence rather than a guarantee.
use crate::service::MiniAppService;

/// Router state: the mini-app service (a cheap `Arc` handle).
#[derive(Clone)]
pub struct MiniAppRouterState {
    pub service: MiniAppService,
}

impl MiniAppRouterState {
    pub fn new(service: MiniAppService) -> Self {
        Self { service }
    }
}
