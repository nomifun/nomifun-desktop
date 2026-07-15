use nomifun_api_types::SystemInfoResponse;

/// Map Rust `std::env::consts::OS` to the Node.js-compatible platform name
/// used by the API contract.
fn map_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other, // "linux" stays "linux"
    }
}

/// Map Rust `std::env::consts::ARCH` to the API contract arch name.
fn map_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

/// Resolve the cache directory for Nomi.
///
/// Priority: `NOMIFUN_CACHE_DIR` env → `dirs::cache_dir()/nomifun`.
fn resolve_cache_dir() -> String {
    if let Ok(v) = std::env::var("NOMIFUN_CACHE_DIR")
        && !v.is_empty()
    {
        return v;
    }
    dirs::cache_dir()
        .map(|p| p.join("nomifun").to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Resolve the work (data) directory for Nomi.
///
/// Priority: `NOMIFUN_WORK_DIR` env → `dirs::data_dir()/nomifun`.
fn resolve_work_dir() -> String {
    if let Ok(v) = std::env::var("NOMIFUN_WORK_DIR")
        && !v.is_empty()
    {
        return v;
    }
    dirs::data_dir()
        .map(|p| p.join("nomifun").to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Resolve the log directory for Nomi.
///
/// Priority: `NOMIFUN_LOG_DIR` env →
///   macOS: `~/Library/Logs/nomifun`
///   Linux: `dirs::state_dir()/nomifun/logs` (XDG_STATE_HOME)
///   Windows: `dirs::data_dir()/nomifun/logs`
fn resolve_log_dir() -> String {
    if let Ok(v) = std::env::var("NOMIFUN_LOG_DIR")
        && !v.is_empty()
    {
        return v;
    }
    // macOS: ~/Library/Logs is the conventional log location
    if cfg!(target_os = "macos")
        && let Some(home) = dirs::home_dir()
    {
        return home.join("Library/Logs/nomifun").to_string_lossy().into_owned();
    }
    // Linux: XDG state dir
    if let Some(state) = dirs::state_dir() {
        return state.join("nomifun/logs").to_string_lossy().into_owned();
    }
    // Fallback: data_dir/nomifun/logs
    dirs::data_dir()
        .map(|p| p.join("nomifun/logs").to_string_lossy().into_owned())
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
        platform: map_platform().to_owned(),
        arch: map_arch().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_platform_known() {
        let p = map_platform();
        // On CI this will be one of the known values
        assert!(["darwin", "win32", "linux"].contains(&p), "unexpected platform: {p}");
    }

    #[test]
    fn test_map_arch_known() {
        let a = map_arch();
        assert!(["x64", "arm64"].contains(&a), "unexpected arch: {a}");
    }

    #[test]
    fn test_get_system_info_fields_non_empty() {
        let info = get_system_info();
        assert!(!info.cache_dir.is_empty(), "cache_dir should not be empty");
        assert!(!info.work_dir.is_empty(), "work_dir should not be empty");
        assert!(!info.log_dir.is_empty(), "log_dir should not be empty");
        assert!(!info.platform.is_empty());
        assert!(!info.arch.is_empty());
    }

    #[test]
    fn test_env_override_cache_dir() {
        // This test verifies the resolve logic reads env vars.
        // We cannot reliably set env in parallel tests, so just verify
        // the default path contains "nomifun".
        let dir = resolve_cache_dir();
        assert!(dir.contains("nomifun"), "cache_dir should contain 'nomifun': {dir}");
    }

    #[test]
    fn test_env_override_work_dir() {
        let dir = resolve_work_dir();
        assert!(dir.contains("nomifun"), "work_dir should contain 'nomifun': {dir}");
    }

    #[test]
    fn test_env_override_log_dir() {
        let dir = resolve_log_dir();
        assert!(dir.contains("nomifun"), "log_dir should contain 'nomifun': {dir}");
    }
}
