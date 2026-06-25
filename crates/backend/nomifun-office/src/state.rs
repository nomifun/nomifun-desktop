use std::path::PathBuf;
use std::sync::Arc;

use crate::conversion::ConversionService;
use crate::proxy::ProxyService;
use crate::snapshot::SnapshotService;
use crate::star_office::StarOfficeDetector;
use crate::watch_manager::OfficecliWatchManager;

#[derive(Clone)]
pub struct OfficeRouterState {
    pub watch_manager: Arc<OfficecliWatchManager>,
    pub snapshot_service: Arc<SnapshotService>,
    pub star_office_detector: Arc<StarOfficeDetector>,
    pub conversion_service: Arc<ConversionService>,
    pub proxy_service: Arc<ProxyService>,
    pub allowed_roots: Vec<PathBuf>,
}
