//! Scheduled job engine: cron scheduler, executor, and lifecycle event emitter.
pub mod agent_schedule;
pub mod busy_guard;
pub mod error;
pub mod events;
pub mod executor;
pub mod prompt;
pub mod routes;
pub mod scheduler;
mod session_port;
pub mod service;
pub mod sink;
pub mod skill_file;
pub mod state;
pub mod types;

pub use events::CronEventEmitter;
pub use agent_schedule::{
    AUTOMATION_SCHEDULE_ACTION_IDS, AUTOMATION_SCHEDULE_MODULE_ID, SCHEDULE_CREATE_ACTION_ID,
    SCHEDULE_DELETE_ACTION_ID, SCHEDULE_LIST_ACTION_ID, SCHEDULE_UPDATE_ACTION_ID,
    SCHEDULER_DELETE_OPERATION, SCHEDULER_READ_OPERATION, SCHEDULER_RESOURCE_KIND,
    SCHEDULER_RESOURCE_OPERATIONS, SCHEDULER_WRITE_OPERATION, ScheduleAction,
    ScheduleActionContext, ScheduleActionError, ScheduleActionOwner, ScheduleAuthority,
    ScheduleCreateInput, ScheduleDeleteInput, ScheduleExternalActionStatus, ScheduleListInput,
    ScheduleListOutput, ScheduleMutationOutput, ScheduleResourceBinding,
    ScheduleResourceOperation, ScheduleResourceSelection, ScheduleUpdateInput,
};
pub use routes::cron_routes;
pub use session_port::{
    CronRuntimePreparationRequest, CronScheduledSession, CronScheduledSessionLookup,
    CronSessionCronBindingRequest, CronSessionHandle, CronSessionLookup, CronSessionPort,
    CronSessionProjection, CronPreparedTurnDelivery, CronTurnDelivery, CronTurnDeliveryQuery,
    CronTurnMessage, CronTurnReceiptState, CronTurnReconciliation,
    CronTurnReconciliationRequest, CronTurnReceiptQuery, CronTurnRequest,
    CronTurnRuntimeOverlay, CronTurnRuntimePreparation,
};
pub use service::{
    CronAgentPresetResolver, CronBackgroundTaskRegistrar, CronEmbeddedCommandResult,
    CronEmbeddedCreateCommand, CronEmbeddedDeleteCommand, CronEmbeddedMutationRequest,
    CronEmbeddedMutationWaiter, CronEmbeddedUpdateCommand,
};
pub use state::CronRouterState;
