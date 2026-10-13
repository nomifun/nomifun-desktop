//! Native macOS adapter using public system WKWebView APIs on AppKit's main thread.
#![cfg(target_os = "macos")]

pub mod engine;
mod callbacks;
mod interactions;
mod view;
mod identity;
mod navigation;
mod shortcuts;
pub use shortcuts::BrowserShortcutAction;
pub use identity::{report as identity_report, runtime_info};
