//! Router state for the crawl domain.

use std::sync::Arc;

use crate::service::CrawlService;

#[derive(Clone)]
pub struct CrawlRouterState {
    pub service: Arc<CrawlService>,
}

impl CrawlRouterState {
    pub fn new(service: Arc<CrawlService>) -> Self {
        Self { service }
    }
}
