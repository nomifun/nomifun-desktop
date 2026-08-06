//! Real implementations of the [`crate::services`] seams.
//!
//! Everything that knows about providers, the invoke layer, conversations and
//! companion profiles lives here — the pipelines above never do.

pub mod dispatcher;
pub mod speech;

pub use dispatcher::{RobotConversationBackend, RobotDispatcher};
pub use speech::{
    CompanionSlotReader, PreferenceReader, ProviderCredentials, ProviderRowReader, RobotSpeech,
    VISION_TIMEOUT_SECS,
};
