use std::sync::Arc;

use nomifun_conversation::ConversationService;

use crate::service::CronService;

#[derive(Clone)]
pub struct CronRouterState {
    pub cron_service: Arc<CronService>,
    pub conversation_service: ConversationService,
}
