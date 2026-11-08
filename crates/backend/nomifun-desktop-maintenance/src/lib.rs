//! Application startup, authenticated shutdown and recoverable package updates.
//! This crate has no device, audio, model, Agent or desktop window dependency.
pub mod desktop_control;
pub mod ipc;
pub mod legacy_service;
pub mod maintenance;
