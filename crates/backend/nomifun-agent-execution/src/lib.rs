//! Persistent collaboration for Nomi Agents.
//!
//! The crate exposes one application facade, [`AgentExecutionEngine`]. A
//! participant is an immutable Agent configuration snapshot, a step is a DAG
//! node, and an attempt is one concrete invocation. Planning, routing and
//! scheduling are internal implementation strategies rather than product or
//! integration concepts.

mod attempt_runner;
mod control_steps;
mod conversation_effect;
mod domain_mapper;
mod engine;
mod event_publisher;
mod participant_resolver;
mod participant_router;
mod plan_materializer;
mod planner;
mod production;
mod routes;
mod scheduler;
mod template_routes;

pub use engine::AgentExecutionEngine;
pub use production::AgentExecutionEngineConfig;
pub use routes::agent_execution_routes;
pub use template_routes::agent_execution_template_routes;
