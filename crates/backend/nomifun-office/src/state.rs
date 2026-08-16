use std::path::PathBuf;
use std::sync::Arc;

use crate::proxy::ProxyService;
use crate::snapshot::SnapshotService;
use crate::watch_manager::OfficecliWatchManager;

#[derive(Clone)]
pub struct OfficeRouterState {
    pub watch_manager: Arc<OfficecliWatchManager>,
    pub snapshot_service: Arc<SnapshotService>,
    pub proxy_service: Arc<ProxyService>,
    pub allowed_roots: Vec<PathBuf>,
}
