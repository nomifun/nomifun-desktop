//! 客服独立域 (customer-service domain).
//!
//! A standalone domain for serving strangers over IM channels. It shares NO
//! concepts with the desktop companion/conversation system: dialogues are the
//! domain's own aggregate, replies are produced by a disposable one-shot
//! engine session whose tool registry is fixed at construction time to three
//! read-only tools.

pub mod service;

pub use service::CustomerServiceService;
