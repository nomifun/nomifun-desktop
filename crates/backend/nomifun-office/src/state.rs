use crate::{OfficePreviewService, SnapshotService};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone)]
pub struct OfficeRouterState {
    pub preview_service: Arc<OfficePreviewService>,
    pub snapshot_service: Arc<SnapshotService>,
    pub allowed_roots: Vec<PathBuf>,
}
