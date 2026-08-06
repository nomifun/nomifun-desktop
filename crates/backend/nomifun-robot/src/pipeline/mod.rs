//! The two audio pipelines and the text plumbing between them.

pub mod downlink;
pub mod sentence;
pub mod uplink;

pub use downlink::{DownlinkPacer, PRIME_FRAMES, encode_for_downlink};
pub use sentence::{EMOTIONS, SentenceSplitter, normalize_emotion, strip_emotion};
pub use uplink::{MAX_UTTERANCE_MS, UplinkOutcome, UplinkPipeline};
