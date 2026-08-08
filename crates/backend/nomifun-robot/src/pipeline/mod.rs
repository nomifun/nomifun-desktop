//! The two audio pipelines and the text plumbing between them.

pub mod downlink;
pub mod sentence;
pub mod uplink;

pub use downlink::{DownlinkPacer, PRIME_FRAMES, encode_for_downlink};
pub use sentence::{
    EMOTIONS, SentenceSplitter, normalize_emotion, sanitize_for_speech, strip_emotion,
    strip_emotion_markers,
};
pub use uplink::{MAX_UTTERANCE_MS, UplinkOutcome, UplinkPipeline};
