//! Runtime capability modules shared across agent managers.
//!
//! These modules provide reusable primitives (backend output/protocol sinks and
//! model-identity reminders) that any agent implementation can compose.

pub(crate) mod backend_output_sink;
pub(crate) mod backend_protocol_sink;
pub mod model_identity_reminder;
