//! Application voice owners. Canonical work is injected through a port.
pub mod registry;
pub mod journal;
pub mod work;
pub mod bridge;
pub mod service;
pub mod endpoint;
mod shared_journal;
pub use shared_journal::SharedVoiceJournal;
pub use endpoint::{ProductVoiceEndpoint, VoiceOutputFrame};
pub use registry::*;
pub use journal::*;
pub use work::*;
pub use bridge::*;
pub use service::*;
