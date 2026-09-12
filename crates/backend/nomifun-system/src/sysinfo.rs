use std::path::PathBuf;

use nomifun_api_types::SystemInfoResponse;

/// Map Rust `std::env::consts::OS` to the Node.js-compatible platform name
/// used by the API contract.
pub(crate) fn map_platform(os: &str) -> &str {
    match os {
        "macos" => "darwin",
        "windows" => "win32",
        other => other, // "linux" stays "linux"
    }
}

/// Map Rust `std::env::consts::ARCH` to the API contract arch name.
pub(crate) fn map_arch(arch: &str) -> &str {
    match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

/// Resolve the cache directory for Nomi.
///
/// Priority: `NOMIFUN_CACHE_DIR` env → `dirs::cache_dir()/nomifun`.
fn resolve_cache_dir() -> String {
    resolve_directory(std::env::var("NOMIFUN_CACHE_DIR").ok(), || {
        dirs::cache_dir().map(|p| p.join("nomifun"))
    })
}

/// Resolve the work (data) directory for Nomi.
///
/// Priority: `NOMIFUN_WORK_DIR` env → `dirs::data_dir()/nomifun`.
fn resolve_work_dir() -> String {
    resolve_directory(std::env::var("NOMIFUN_WORK_DIR").ok(), || {
        dirs::data_dir().map(|p| p.join("nomifun"))
    })
}

/// Resolve the log directory for Nomi.
///
/// Priority: `NOMIFUN_LOG_DIR` env →
///   macOS: `~/Library/Logs/nomifun`
///   Linux: `dirs::state_dir()/nomifun/logs` (XDG_STATE_HOME)
///   Windows: `dirs::data_dir()/nomifun/logs`
fn resolve_log_dir() -> String {
    resolve_directory(std::env::var("NOMIFUN_LOG_DIR").ok(), || {
        // macOS: ~/Library/Logs is the conventional log location
        if cfg!(target_os = "macos")
            && let Some(home) = dirs::home_dir()
        {
            return Some(home.join("Library/Logs/nomifun"));
        }
        dirs::state_dir()
            .or_else(dirs::data_dir)
            .map(|p| p.join("nomifun/logs"))
    })
}

fn resolve_directory(
    override_value: Option<String>,
    fallback: impl FnOnce() -> Option<PathBuf>,
) -> String {
    if let Some(value) = override_value.filter(|value| !value.is_empty()) {
        return value;
    }
    fallback()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn resolve_storage_generation() -> String {
    std::env::var("NOMIFUN_STORAGE_GENERATION")
        .unwrap_or_else(|_| "uninitialized".to_owned())
}

/// Build the system info response from the current runtime environment.
pub fn get_system_info() -> SystemInfoResponse {
    SystemInfoResponse {
        cache_dir: resolve_cache_dir(),
        work_dir: resolve_work_dir(),
        log_dir: resolve_log_dir(),
        storage_generation: resolve_storage_generation(),
        platform: map_platform(std::env::consts::OS).to_owned(),
        arch: map_arch(std::env::consts::ARCH).to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_platform_known() {
        for (os, expected) in [("macos", "darwin"), ("windows", "win32"), ("linux", "linux"), ("freebsd", "freebsd")] {
            assert_eq!(map_platform(os), expected);
        }
    }

    #[test]
    fn test_map_arch_known() {
        for (arch, expected) in [("x86_64", "x64"), ("aarch64", "arm64"), ("riscv64", "riscv64")] {
            assert_eq!(map_arch(arch), expected);
        }
    }

    #[test]
    fn directory_override_is_preserved_without_resolving_defaults() {
        for value in ["custom/cache", "custom/work", "custom/logs", "  "] {
            assert_eq!(resolve_directory(Some(value.to_owned()), || panic!("override wins")), value);
        }
    }

    #[test]
    fn absent_or_empty_directory_override_uses_the_fallback() {
        for value in [None, Some(String::new())] {
            assert_eq!(
                resolve_directory(value.clone(), || Some(PathBuf::from("synthetic-directory"))),
                "synthetic-directory"
            );
            assert_eq!(resolve_directory(value, || None), "");
        }
    }
}
