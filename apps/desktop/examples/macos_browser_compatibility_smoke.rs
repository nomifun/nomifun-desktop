//! Real WKWebView compatibility smoke with disposable loopback pages.
//! Run: cargo run -p nomifun-desktop --example macos_browser_compatibility_smoke
//! --hold-ui waits for a physical click on the popup link with user input unlocked.
//! --live-sites adds a 60-second read-only Bilibili/Baidu entry check.
//! --media-permissions tests real website permission denial/retirement only.
//! --media-permissions --media-user-ready waits up to 60 seconds per case for a trusted Start click.
//! It never creates an app backend, reads a user database, or uses CDP/evaluate.

#[cfg(target_os = "macos")]
#[path = "../src/macos_appkit_main_loop.rs"]
mod macos_appkit_main_loop;
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
#[path = "support/macos_browser_compatibility_permissions.rs"]
mod permissions;
#[cfg(target_os = "macos")]
#[path = "../src/macos_development_launch.rs"]
mod application_launch;

#[cfg(target_os = "macos")]
fn main() {
    use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
    let hold_ui = std::env::args().any(|arg| arg == "--hold-ui");
    let live_sites = std::env::args().any(|arg| arg == "--live-sites");
    let media_user_ready = std::env::args().any(|arg| arg == "--media-user-ready");
    let media_permissions = std::env::args().any(|arg| arg == "--media-permissions");
    if media_user_ready && !media_permissions {
        eprintln!("--media-user-ready requires explicit --media-permissions");
        std::process::exit(2);
    }
    // The fixed audit scope admits LaunchServices before the loopback worker,
    // storage, Tauri runtime, or any native browser object can be created.
    let mut bootstrap = match application_launch::prepare_application(application_launch::ApplicationScope::BrowserMediaAudit) {
        Ok(bootstrap) => bootstrap,
        Err(error) => {
            eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL application admission failed: {error}");
            std::process::exit(1);
        }
    };
    let fixture = Arc::new(fixture::Fixture::new().expect("disposable loopback fixture"));
    let temporary = tempfile::tempdir().expect("disposable smoke evidence directory");
    let report_path = temporary.path().join("compatibility-report.json");
    let success = Arc::new(AtomicBool::new(false));
    let native_cleanup = Arc::new(AtomicBool::new(false));
    let evidence = Arc::new(Mutex::new(None));
    let completed = success.clone();
    let completed_evidence = evidence.clone();
    let completed_cleanup = native_cleanup.clone();
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
                let base = if media_user_ready { 450 } else if hold_ui && live_sites { 240 } else { 150 };
                tokio::time::sleep(std::time::Duration::from_secs(base + if media_permissions { 120 } else { 0 })).await;
                eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL native runtime timed out");
                watchdog.exit(1);
            });
            tauri::async_runtime::spawn(async move {
                let result = probe::verify(&handle, &worker_fixture, hold_ui, live_sites, media_permissions, media_user_ready, &completed_cleanup).await;
                // Release this worker's fixture owner before dispatching exit;
                // main can then prove its server Drop/joins actually happened.
                drop(worker_fixture);
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
    let fixture_cleanup = match Arc::try_unwrap(fixture) {
        Ok(fixture) => { drop(fixture); true }, // Drop stops and joins its owned HTTP workers.
        Err(fixture) => {
            drop(fixture);
            eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL fixture work remained retained at application exit");
            false
        }
    };
    let evidence_cleanup = match temporary.close() {
        Ok(()) => true,
        Err(_) => { eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL disposable evidence cleanup failed"); false },
    };
    let cleanup_verified = native_cleanup.load(Ordering::SeqCst) && fixture_cleanup && evidence_cleanup;
    let mut code = if exit == 0 && success.load(Ordering::SeqCst) && cleanup_verified { 0 } else { 1 };
    if let Some(bootstrap) = &mut bootstrap {
        if let Err(error) = bootstrap.report_exit(code, cleanup_verified) {
            eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL application shutdown receipt failed: {error}");
            code = 1;
        }
    }
    if code == 0 {
        let mut report = evidence.lock().unwrap().take().expect("smoke evidence");
        report["disposable_evidence_cleanup"] = serde_json::json!(true);
        report["native_cleanup_verified"] = serde_json::json!(true);
        if live_sites { println!("MACOS_BROWSER_COMPATIBILITY_SMOKE_CORE_PASS {report}"); }
        else { println!("MACOS_BROWSER_COMPATIBILITY_SMOKE_PASS {report}"); }
    }
    std::process::exit(code);
}

#[cfg(not(target_os = "macos"))]
fn main() { eprintln!("macos_browser_compatibility_smoke requires macOS 14 or newer"); std::process::exit(1); }
