//! The two audio pipelines and the text plumbing between them.

pub mod downlink;
pub mod sentence;
pub mod uplink;

pub use downlink::{DownlinkPacer, PRIME_FRAMES, encode_for_downlink};
pub use sentence::{
    SentenceSplitter, sanitize_for_display, sanitize_for_speech, strip_stage_directions,
};
pub use uplink::{MAX_UTTERANCE_MS, UplinkOutcome, UplinkPipeline};
