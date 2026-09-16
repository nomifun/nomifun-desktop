//! Browser v2 primitives: native semantics, explicit input, isolated page execution,
//! and connection-only control of an existing user browser. No legacy engine/Host/Lane.
pub mod attached_browser;
mod cleanup;
pub mod download;
mod engine;
pub mod frame_geometry;
pub mod headless_page;
mod injected;
pub mod input;
#[cfg(feature = "conformance")]
pub mod launch;
#[cfg(not(feature = "conformance"))]
mod launch;
pub mod native_semantic;
#[cfg(feature = "conformance")]
pub mod profile;
#[cfg(not(feature = "conformance"))]
mod profile;
pub mod redact;
mod session;
mod switches;
#[cfg(feature = "conformance")]
pub mod transport;
#[cfg(not(feature = "conformance"))]
mod transport;
pub use engine::BrowserError;
