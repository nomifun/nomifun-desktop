//! The two audio pipelines and the text plumbing between them.

pub mod sentence;

pub use sentence::{EMOTIONS, SentenceSplitter, normalize_emotion, strip_emotion};
