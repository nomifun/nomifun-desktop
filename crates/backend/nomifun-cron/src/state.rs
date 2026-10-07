use std::sync::Arc;

use crate::service::CronService;

#[derive(Clone)]
pub struct CronRouterState {
    pub cron_service: Arc<CronService>,
}
