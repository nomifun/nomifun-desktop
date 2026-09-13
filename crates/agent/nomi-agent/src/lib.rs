// Core agent infrastructure: engine, session, tool execution, output sinks.

pub mod agents_md;
pub mod bootstrap;
pub mod cache_diagnostics;
pub mod commands;
pub mod compact;
pub mod companion_tools;
pub mod computer_history_tools;
pub mod confirm;
pub mod context;
pub mod context_contributor;
pub mod cron_tools;
pub mod memory_tools;
pub mod engine;
pub mod goal;
pub mod knowledge_tools;
pub mod loop_guard;
pub mod tool_execution;
pub mod output;
pub mod plan;
pub mod requirement_tools;
pub mod round;
pub mod ssh_backend;
pub mod ssh_tools;
pub mod session;
pub mod skill_tool;
pub mod summon_tools;
mod local_agent_invocation;
mod local_delegation_progress;
mod local_delegate_tool;

// Re-export the skills crate so existing callers (nomi-cli, tests) can use
// `nomi_agent::skills::` without changing their import paths.
pub use nomi_skills as skills;

pub use knowledge_tools::{KnowledgeHit, KnowledgeReadTool, KnowledgeRetrievalSink, KnowledgeSearchTool};
pub use computer_history_tools::{ComputerHistorySink, register_computer_history_tools};
