//! Browser resources owned by the application composition root.
//!
//! ACP stdio is an authenticated proxy and must not discover, allocate, or
//! launch browser resources. Resolve packaged resources here, while composing
//! the one managed browser host used by the application process.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Host-provided override for the packaged Chrome-for-Testing directory.
///
/// The desktop shell resolves Tauri's platform resource directory (the only
/// authority on where bundled resources actually land) before it boots the
/// backend, and publishes `<resource_dir>/chrome-for-testing` here. This crate
/// deliberately has no Tauri dependency, so an environment variable is the
/// seam between the two.
pub const BUNDLED_CHROME_DIR_ENV: &str = "NOMIFUN_BUNDLED_CHROME_DIR";

/// Resolve the optional packaged Chrome-for-Testing resource directory.
pub(crate) fn bundled_chrome_dir() -> Option<PathBuf> {
    let exe_dir = std::env::current_exe()
        .ok()?
        .canonicalize()
        .ok()?
        .parent()?
        .to_path_buf();
    resolve_bundled_chrome_dir(&exe_dir, std::env::var_os(BUNDLED_CHROME_DIR_ENV))
}

/// Candidate order (first existing directory wins):
/// 1. The host override (Tauri resource-dir resolution, see
///    [`BUNDLED_CHROME_DIR_ENV`]) — authoritative when the host set it.
/// 2. `<exe_dir>/chrome-for-testing` — Windows NSIS layout (resources beside
///    the executable) and bare/dev layouts.
/// 3. `<exe_dir>/../Resources/chrome-for-testing` — macOS `.app` bundles put
///    the executable in `Contents/MacOS` while bundled resources land in
///    `Contents/Resources`, so an exe-relative sibling lookup alone can never
///    find a packaged Chrome there (F48). Harmless elsewhere.
fn resolve_bundled_chrome_dir(
    exe_dir: &Path,
    env_override: Option<OsString>,
) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(dir) = env_override.filter(|value| !value.is_empty()) {
        candidates.push(PathBuf::from(dir));
    }
    candidates.push(exe_dir.join("chrome-for-testing"));
    if let Some(parent) = exe_dir.parent() {
        candidates.push(parent.join("Resources").join("chrome-for-testing"));
    }
    candidates.into_iter().find(|dir| dir.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_override_wins_when_it_exists() {
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join("bin");
        let override_dir = root.path().join("resources/chrome-for-testing");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&override_dir).unwrap();
        std::fs::create_dir_all(exe_dir.join("chrome-for-testing")).unwrap();

        assert_eq!(
            resolve_bundled_chrome_dir(&exe_dir, Some(override_dir.clone().into_os_string())),
            Some(override_dir)
        );
    }

    #[test]
    fn missing_env_override_falls_back_to_exe_layouts() {
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join("bin");
        let beside_exe = exe_dir.join("chrome-for-testing");
        std::fs::create_dir_all(&beside_exe).unwrap();

        assert_eq!(
            resolve_bundled_chrome_dir(
                &exe_dir,
                Some(root.path().join("does-not-exist").into_os_string()),
            ),
            Some(beside_exe)
        );
    }

    #[test]
    fn macos_app_bundle_layout_resolves_contents_resources() {
        // Foo.app/Contents/MacOS/<exe> with resources in Contents/Resources.
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join("Foo.app/Contents/MacOS");
        let resources = root.path().join("Foo.app/Contents/Resources/chrome-for-testing");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(&resources).unwrap();

        assert_eq!(
            resolve_bundled_chrome_dir(&exe_dir, None),
            Some(resources)
        );
    }

    #[test]
    fn no_candidate_yields_none() {
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join("bin");
        std::fs::create_dir_all(&exe_dir).unwrap();

        assert_eq!(resolve_bundled_chrome_dir(&exe_dir, None), None);
    }
}
