#[cfg(unix)]
fn main() {
    let expected = std::env::args_os().nth(1).expect("expected a fixture PATH directory");
    assert!(std::env::var_os("SHELL").is_none(), "fixture must mirror a GUI launch without SHELL");
    assert_eq!(nomi_process_runtime::platform_lifecycle_metrics().poller_threads, 0);
    let start = std::time::Instant::now();
    // SAFETY: a new standalone main process, before runtime/storage/UI workers.
    let merged = unsafe { nomifun_runtime::enhance_process_path() };
    assert!(std::env::split_paths(&merged).any(|path| path.as_os_str() == expected),
        "account login shell did not load the fixture profile PATH");
    assert!(start.elapsed() < std::time::Duration::from_secs(7), "profile descendants stalled startup");
    assert_eq!(nomi_process_runtime::platform_lifecycle_metrics().poller_threads, 0,
        "startup probe leaked lifecycle worker threads into the host");
    println!("GUI startup without SHELL loaded account profile and bounded background output");
}

#[cfg(not(unix))]
fn main() {}
