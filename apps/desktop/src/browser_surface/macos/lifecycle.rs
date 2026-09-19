//! Production CEF bundle discovery and process-wide lifetime.
//!
//! A development binary outside a macOS application bundle is intentionally
//! unavailable. The native fixture has its own explicit bundle contract; the
//! product host accepts only the packaged framework/helper layout below.

use std::path::Path;
use std::sync::Arc;

use nomifun_browser_macos::engine::{Engine, Paths};

const FRAMEWORK_NAME: &str = "Chromium Embedded Framework.framework";
const HELPER_NAME: &str = "NomiFun Helper";

pub(crate) fn initialize(data_dir: &Path) -> Result<Arc<Engine>, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "macOS CEF executable path is unavailable".to_owned())?;
    let paths = packaged_paths(&executable, data_dir)?;
    Engine::initialize(paths)
}

fn packaged_paths(executable: &Path, data_dir: &Path) -> Result<Paths, String> {
    let macos = executable
        .parent()
        .filter(|path| path.file_name().is_some_and(|name| name == "MacOS"))
        .ok_or_else(|| "macOS CEF is available only from a packaged application".to_owned())?;
    let contents = macos
        .parent()
        .filter(|path| path.file_name().is_some_and(|name| name == "Contents"))
        .ok_or_else(|| "macOS CEF application bundle layout is invalid".to_owned())?;
    let bundle = contents
        .parent()
        .filter(|path| path.extension().is_some_and(|extension| extension == "app"))
        .ok_or_else(|| "macOS CEF application bundle root is invalid".to_owned())?;
    let frameworks = contents.join("Frameworks");
    Ok(Paths {
        framework: frameworks.join(FRAMEWORK_NAME),
        helper: frameworks
            .join(format!("{HELPER_NAME}.app"))
            .join("Contents/MacOS")
            .join(HELPER_NAME),
        main_bundle: bundle.to_path_buf(),
        // BrowserProfileStore derives persistent resources below browser-v3.
        // CEF's root_cache_path must be their canonical ancestor.
        data_root: data_dir.join("browser-v3"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_only_the_owned_product_bundle_layout() {
        let root = tempfile::tempdir().unwrap();
        let executable = root
            .path()
            .join("NomiFun.app/Contents/MacOS/nomifun-desktop");
        let data = root.path().join("data");
        let paths = packaged_paths(&executable, &data).unwrap();
        assert_eq!(
            paths.framework,
            root.path().join(
                "NomiFun.app/Contents/Frameworks/Chromium Embedded Framework.framework"
            )
        );
        assert_eq!(
            paths.helper,
            root.path().join(
                "NomiFun.app/Contents/Frameworks/NomiFun Helper.app/Contents/MacOS/NomiFun Helper"
            )
        );
        assert_eq!(paths.main_bundle, root.path().join("NomiFun.app"));
        assert_eq!(paths.data_root, data.join("browser-v3"));

        assert!(packaged_paths(&root.path().join("nomifun-desktop"), &data).is_err());
        assert!(packaged_paths(
            &root.path().join("NomiFun/Contents/MacOS/nomifun-desktop"),
            &data
        )
        .is_err());
    }
}
