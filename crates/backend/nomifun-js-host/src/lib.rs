//! Lazy shared JavaScript Extension Host for Phase N1.
//!
//! The host owns process generations and private stdio transport only. Package
//! identity, contribution identity, and executable provenance remain canonical
//! `nomifun-agent-contracts` values supplied by the caller.
//!
//! Tool and Context exports receive a request-scoped AbortSignal. Dropping a
//! Rust invocation wait requests cancellation through the generation's bounded
//! control lane; an acknowledgment is not a terminal result or rollback. The
//! original request remains supervised until it settles or reaches its deadline.
//! Plugin exports must yield and release their request-local work when aborted.

#![forbid(unsafe_code)]

mod error;
mod dependencies;
mod outbound;
mod supervisor;

pub use error::*;
pub use dependencies::ExtensionHostDependencyCaller;
pub use supervisor::*;
