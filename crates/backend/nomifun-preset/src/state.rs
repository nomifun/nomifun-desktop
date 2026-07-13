use std::sync::Arc;
use crate::service::PresetService;

#[derive(Clone)]
pub struct PresetRouterState { pub service: Arc<PresetService> }
