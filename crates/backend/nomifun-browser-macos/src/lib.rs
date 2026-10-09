//! Native macOS adapter using public system WKWebView APIs on AppKit's main thread.
#![cfg(target_os = "macos")]

pub mod engine;
mod callbacks;
mod interactions;
mod view;
