//! Browser resources owned by the application composition root.
//!
//! ACP stdio is an authenticated proxy and must not discover, allocate, or
//! launch browser resources. Resolve packaged resources here, while composing
//! the one managed browser host used by the application process.

use std::path::PathBuf;

/// Resolve the optional packaged Chrome-for-Testing resource directory.
pub(crate) fn bundled_chrome_dir() -> Option<PathBuf> {
    let dir = std::env::current_exe()
        .ok()?
        .canonicalize()
        .ok()?
        .parent()?
        .join("chrome-for-testing");
    dir.is_dir().then_some(dir)
}
