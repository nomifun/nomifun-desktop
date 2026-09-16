//! Compatibility names over the engine-neutral process owner. Production
//! platform adapters depend on engine-core, not the Coding strategy crate.
//! Process methods return EngineProcessError, convertible to CodingEngineError.
pub use nomifun_engine_core::{
    EngineCleanupReport as CodingCleanupReport, EngineProcessOutput as CodingProcessOutput,
    EngineProcessPoll as CodingProcessPoll, EngineProcessRequest as CodingProcessRequest,
    EngineProcessSession as CodingProcessSession, EngineProcessTransport as CodingProcessTransport,
    ManagedEngineProcessOwner as ManagedCodingProcessOwner,
};
