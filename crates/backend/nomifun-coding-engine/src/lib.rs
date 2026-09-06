//! Isolated in-process Coding Agent execution engine.
//!
//! This crate is intentionally not wired into the current application
//! composition root. It provides the second execution engine behind narrow
//! NomiFun ports so it can be developed and validated independently before
//! remote-main integration.

#![forbid(unsafe_code)]

mod engine;
mod error;
mod events;
mod model;
mod tool;
mod turn;

pub use engine::{
    AgentEngine, CodingEngine, CodingEngineBuild, CodingEngineCatalog, CodingEngineChannel,
    CodingEngineSelector, CodingEngineSession, CodingRuntimeProfile, EngineBinding,
    EngineBuildId, EngineFamilyId,
};
pub use error::CodingEngineError;
pub use events::{CodingEngineEvent, CodingEventSink, NoopCodingEventSink};
pub use model::{BrokerCodingModelPort, CodingModelPort, CodingModelStream};
pub use tool::{
    input_schema_digest, CodingEffectClass, CodingToolBinding, CodingToolInvocation,
    CodingToolInvoker, CodingToolPlan, CodingToolResult,
};
pub use turn::{CodingTurnRequest, CodingTurnResult, CodingTurnTerminal};
