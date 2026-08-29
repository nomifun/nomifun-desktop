//! Provider-neutral Agent chat broker and stateless local Responses bridge.
//!
//! The broker owns exact route/credential resolution and is the only model
//! retry/failover authority. Protocol adapters and the Responses bridge perform
//! one transport attempt only, and a route becomes immutable when the first
//! semantic model event is emitted.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod broker;
pub mod contracts;
pub mod ports;
pub mod recorded;
pub mod responses_bridge;

pub use adapter::*;
pub use broker::*;
pub use contracts::*;
pub use ports::*;
pub use recorded::*;
pub use responses_bridge::*;
