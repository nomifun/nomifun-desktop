use crate::service::FleetService;

/// Router state for the orchestration endpoints.
///
/// This is the minimal shell consumed by the (future) `orchestrator_routes` in
/// Task 7. Task 6 will add a `workspace: WorkspaceService` field; for now it
/// carries only the fleet service.
#[derive(Clone)]
pub struct OrchestratorRouterState {
    pub fleet: FleetService,
}

impl OrchestratorRouterState {
    pub fn new(fleet: FleetService) -> Self {
        Self { fleet }
    }
}
