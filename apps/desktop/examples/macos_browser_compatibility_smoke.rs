//! Real WKWebView compatibility smoke with disposable loopback pages.
//! Run: cargo run -p nomifun-desktop --example macos_browser_compatibility_smoke
//! --hold-ui waits for a physical click on the popup link with user input unlocked.
//! --live-sites adds a 60-second read-only Bilibili/Baidu entry check.
//! It never creates an app backend, reads a user database, or uses CDP/evaluate.

#[cfg(target_os = "macos")]
#[path = "../src/browser_surface/macos/mod.rs"]
mod macos;
#[cfg(target_os = "macos")]
#[path = "support/macos_browser_compatibility_fixture.rs"]
mod fixture;
#[cfg(target_os = "macos")]
#[path = "support/macos_browser_compatibility_probe.rs"]
mod probe;
#[cfg(target_os = "macos")]
#[path = "support/macos_browser_compatibility_live.rs"]
mod live;
#[cfg(target_os = "macos")]
#[path = "support/macos_browser_compatibility_downloads.rs"]
mod downloads;

#[cfg(target_os = "macos")]
fn main() {
    use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
    let fixture = Arc::new(fixture::Fixture::new().expect("disposable loopback fixture"));
    let temporary = tempfile::tempdir().expect("disposable smoke evidence directory");
    let report_path = temporary.path().join("compatibility-report.json");
    let hold_ui = std::env::args().any(|arg| arg == "--hold-ui");
    let live_sites = std::env::args().any(|arg| arg == "--live-sites");
    let success = Arc::new(AtomicBool::new(false));
    let evidence = Arc::new(Mutex::new(None));
    let completed = success.clone();
    let completed_evidence = evidence.clone();
    let worker_fixture = fixture.clone();
    let app = tauri::Builder::default()
        .invoke_handler(|invoke| {
            invoke.resolver.reject("Compatibility fixture has no application commands");
            true
        })
        .setup(move |app| {
            tauri::window::WindowBuilder::new(app, "main")
                .title("NomiFun macOS Browser Compatibility Smoke")
                .inner_size(880.0, 600.0)
                .min_inner_size(880.0, 600.0)
                .build()?;
            let handle = app.handle().clone();
            let watchdog = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(if hold_ui && live_sites { 240 } else { 150 })).await;
                eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL native runtime timed out");
                watchdog.exit(1);
            });
            tauri::async_runtime::spawn(async move {
                let result = probe::verify(&handle, &worker_fixture, hold_ui, live_sites).await;
                match result {
                    Ok(report) => {
                        if std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).is_ok() {
                            *completed_evidence.lock().unwrap() = Some(report);
                            completed.store(true, Ordering::SeqCst);
                        } else {
                            eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL disposable report write failed");
                        }
                    }
                    Err(error) => eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL {error}"),
                }
                handle.exit(if completed.load(Ordering::SeqCst) { 0 } else { 1 });
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("isolated compatibility application");
    let exit = app.run_return(|_, _| {});
    drop(fixture);
    if temporary.close().is_err() {
        eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL disposable evidence cleanup failed");
        std::process::exit(1);
    }
    if exit != 0 || !success.load(Ordering::SeqCst) { std::process::exit(1); }
    let mut report = evidence.lock().unwrap().take().expect("smoke evidence");
    report["disposable_evidence_cleanup"] = serde_json::json!(true);
    if live_sites { println!("MACOS_BROWSER_COMPATIBILITY_SMOKE_CORE_PASS {report}"); }
    else { println!("MACOS_BROWSER_COMPATIBILITY_SMOKE_PASS {report}"); }
}

#[cfg(not(target_os = "macos"))]
fn main() { eprintln!("macos_browser_compatibility_smoke requires macOS 14 or newer"); std::process::exit(1); }
