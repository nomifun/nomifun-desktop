use crate::service::{FleetService, WorkspaceService};

/// Router state for the orchestration endpoints.
///
/// Consumed by the (future) `orchestrator_routes` in Task 7. Carries the fleet
/// and workspace services.
#[derive(Clone)]
pub struct OrchestratorRouterState {
    pub fleet: FleetService,
    pub workspace: WorkspaceService,
}

impl OrchestratorRouterState {
    pub fn new(fleet: FleetService, workspace: WorkspaceService) -> Self {
        Self { fleet, workspace }
    }
}
