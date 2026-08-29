//! Fresh-v4 data-root cutover, initialization, and recovery.

#![forbid(unsafe_code)]

mod coordinator;
mod database;
mod error;
mod fault;
mod filesystem;
mod inputs;

pub use coordinator::{
    FRESH_V4_DATABASE_FILE, FRESH_V4_INITIALIZING_MARKER_FILE, FreshV4BootstrapOutcome,
    FreshV4Coordinator, FreshV4RecoveryPhase,
};
pub use error::FreshV4RootError;
pub use fault::{
    FreshV4AccessAudit, FreshV4AccessKind, FreshV4Clock, FreshV4FaultInjector,
    FreshV4FaultPoint, FreshV4QuiescePort, NoAccessAudit, NoFaults, PreServiceQuiesced,
    SystemUtcClock,
};
pub use inputs::{
    application_build_digest, canonical_schema_manifest_digest, official_seed_manifest_digest,
};
pub use nomifun_agent_contracts::{
    FRESH_V4_PARENT_MARKER_FILE, FRESH_V4_READY_MARKER_FILE, FreshV4OperationKind,
    FreshV4ParentOperationMarker, FreshV4ReadyMarker, FreshV4SchemaMetadata,
};

#[cfg(test)]
mod tests;
