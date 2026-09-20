//! Intelligent Decision-Making Mode for canonical AgentSessions.
//!
//! The crate deliberately owns neither a Session nor a model runtime.  It
//! observes bounded Session facts through [`IdmmSessionPort`], applies a
//! deterministic rule policy first, and asks [`IdmmBypassModelPort`] only when
//! an explicitly configured bypass model is required.

#![forbid(unsafe_code)]

mod detector;
mod service;
mod store;

pub use detector::{DecisionClass, DecisionOption, DecisionPrompt, detect_decision};
pub use service::{
    IdmmBypassModelPort, IdmmProgressPhase, IdmmProgressSink, IdmmService,
    IdmmSessionObservation, IdmmSessionPort, ObservedMessage, ObservedMessageRole,
    ObservedTurn, ObservedTurnState,
};
