//! No-op observer backend for non-macOS platforms. The crate compiles
//! everywhere; nothing is sampled and no permission is required.

use async_trait::async_trait;

use crate::observer::{ActivitySample, ObserverBackend};
use crate::service::PermissionState;

pub struct StubBackend;

#[async_trait]
impl ObserverBackend for StubBackend {
    async fn current_sample(&self) -> Option<ActivitySample> {
        None
    }

    fn permission_state(&self) -> PermissionState {
        PermissionState::NotRequired
    }
}
