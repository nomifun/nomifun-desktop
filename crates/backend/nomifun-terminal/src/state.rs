use std::sync::Arc;

use crate::service::TerminalService;

/// Router state for the terminal module.
#[derive(Clone)]
pub struct TerminalRouterState {
    pub terminal_service: Arc<TerminalService>,
}

impl TerminalRouterState {
    pub fn new(terminal_service: Arc<TerminalService>) -> Self {
        Self { terminal_service }
    }
}
