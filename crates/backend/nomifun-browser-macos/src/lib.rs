//! Native macOS adapter using public system WKWebView APIs on AppKit's main thread.
#[cfg(target_os = "macos")]
pub mod engine;
#[cfg(target_os = "macos")]
mod callbacks;
#[cfg(target_os = "macos")]
mod interactions;
#[cfg(target_os = "macos")]
mod view;
#[cfg(target_os = "macos")]
mod identity;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod navigation;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod state;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
mod popup;
#[cfg(target_os = "macos")]
pub use identity::{report as identity_report, runtime_info, BrowserIdentityPolicy};
#[cfg(target_os = "macos")]
mod shortcuts;
#[cfg(target_os = "macos")]
pub use shortcuts::BrowserShortcutAction;
