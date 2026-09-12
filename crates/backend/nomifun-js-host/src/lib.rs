//! Lazy shared JavaScript Extension Host for Phase N1.
//!
//! The host owns process generations and private stdio transport only. Package
//! identity, contribution identity, and executable provenance remain canonical
//! `nomifun-agent-contracts` values supplied by the caller.

#![forbid(unsafe_code)]

mod error;
mod outbound;
mod supervisor;

pub use error::*;
pub use supervisor::*;
