//! Sandboxed CEF subprocess entry point. This process owns no backend or IPC authority.
#[cfg(target_os = "macos")]
fn main() {
    use cef::*;
    if std::env::args().skip(1).eq(["--print-runtime-path"]) {
        if let Some(path) = sys::get_cef_dir() { println!("{}", path.display()); return; }
        std::process::exit(2);
    }
    #[cfg(debug_assertions)] {
        let role = std::env::args().find_map(|arg| arg.strip_prefix("--type=").map(str::to_owned)).unwrap_or_default();
        let role = match role.as_str() { "renderer" => "renderer", "gpu-process" => "gpu", "utility" => "utility", _ => "other" };
        eprintln!("CEF_HELPER_START pid={} role={role}", std::process::id());
    }
    // Renderer/GPU/network processes do not inherit provider credentials or
    // application configuration. This runs before CEF starts any threads.
    let keys: Vec<_> = std::env::vars_os().map(|(key, _)| key).collect();
    for key in keys {
        if !matches!(key.to_str(), Some("PATH" | "HOME" | "TMPDIR" | "LANG" | "LC_ALL" | "USER" | "LOGNAME" | "__CF_USER_TEXT_ENCODING" | "MallocNanoZone")) {
            unsafe { std::env::remove_var(key); }
        }
    }
    let args = args::Args::new();
    // Initialize the Chromium seatbelt context before loading the framework.
    let mut sandbox = sandbox::Sandbox::new();
    sandbox.initialize(args.as_main_args());
    #[cfg(debug_assertions)] eprintln!("CEF_HELPER_SANDBOX pid={}", std::process::id());
    let loader = library_loader::LibraryLoader::new(&std::env::current_exe().expect("helper path"), true);
    if !loader.load() {
        eprintln!("CEF helper could not load its bundled framework");
        std::process::exit(2);
    }
    let _ = api_hash(sys::CEF_API_VERSION_LAST, 0);
    #[cfg(debug_assertions)] eprintln!("CEF_HELPER_EXECUTE pid={}", std::process::id());
    let result = execute_process(Some(args.as_main_args()), None::<&mut App>, std::ptr::null_mut());
    // A helper must never continue as a browser/main process.
    std::process::exit(if result < 0 { 2 } else { result });
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("The CEF helper is only supported on macOS; Windows uses WebView2.");
    std::process::exit(2);
}
