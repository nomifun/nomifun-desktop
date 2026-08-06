//! The two audio pipelines and the text plumbing between them.

pub mod sentence;
pub mod uplink;

pub use sentence::{EMOTIONS, SentenceSplitter, normalize_emotion, strip_emotion};
pub use uplink::{MAX_UTTERANCE_MS, UplinkOutcome, UplinkPipeline};
