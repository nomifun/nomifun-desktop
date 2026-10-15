//! Real WKWebView compatibility smoke with disposable loopback pages.
//! Run: cargo run -p nomifun-desktop --example macos_browser_compatibility_smoke
//! --hold-ui waits for a physical click on the popup link with user input unlocked.
//! --live-sites observes Baidu homepage/search and Bilibili for 60s each.
//! --manual-sites adds physical page input and host Back/Reload/show/hide steps.
//! --identity-ab compares Safari17/native WK on identically prepared stores.
//! --evidence-dir <path> retains JSON and site screenshots in that directory.
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
#[cfg(any(target_os = "macos", test))]
#[path = "support/macos_browser_compatibility_outcome.rs"]
mod outcome;
#[cfg(target_os = "macos")]
#[path = "support/macos_browser_identity_ab.rs"]
mod identity_ab;
#[cfg(target_os = "macos")]
#[path = "support/macos_browser_compatibility_permissions.rs"]
mod permissions;
#[cfg(target_os = "macos")]
#[path = "../src/macos_development_launch.rs"]
mod application_launch;

#[cfg(target_os = "macos")]
fn main() {
    use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
    let mut hold_ui = false;
    let mut live_sites = false;
    let mut manual_sites = false;
    let mut compare_identity = false;
    let mut media_permissions = false;
    let mut media_user_ready = false;
    let mut evidence_dir = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--hold-ui" => hold_ui = true,
            "--live-sites" => live_sites = true,
            "--manual-sites" => { manual_sites = true; live_sites = true; }
            "--identity-ab" => compare_identity = true,
            "--media-permissions" => media_permissions = true,
            "--media-user-ready" => media_user_ready = true,
            "--evidence-dir" => evidence_dir = Some(std::path::PathBuf::from(arguments.next().expect("--evidence-dir needs a path"))),
            application_launch::DESCRIPTOR_ARGUMENT => {
                // The launcher adds this host argument. Admission below owns
                // validation; consume only its paired value in this CLI parser.
                arguments.next().expect("development descriptor needs a path");
            }
            _ => panic!("Unknown compatibility runner option"),
        }
    }
    if media_user_ready && !media_permissions {
        eprintln!("--media-user-ready requires explicit --media-permissions");
        std::process::exit(2);
    }
    if compare_identity && media_permissions {
        eprintln!("--identity-ab and --media-permissions use separate audit runs; run each explicitly");
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
    if let Some(directory) = &evidence_dir { std::fs::create_dir_all(directory).expect("evidence directory"); }
    let report_path = evidence_dir.as_deref().unwrap_or(temporary.path()).join("compatibility-report.json");
    let retained_report_path = evidence_dir.as_ref().map(|_| report_path.clone());
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
                let base = if manual_sites { 960 } else if live_sites { 420 } else { 150 };
                let per_arm = base.max(if media_user_ready { 450 } else { 0 }) + if media_permissions { 120 } else { 0 };
                tokio::time::sleep(std::time::Duration::from_secs(per_arm * if compare_identity { 2 } else { 1 })).await;
                eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL native runtime timed out");
                watchdog.exit(1);
            });
            tauri::async_runtime::spawn(async move {
                let result = if compare_identity {
                    let result = identity_ab::verify(&handle, &worker_fixture, hold_ui, live_sites, manual_sites, evidence_dir.as_deref()).await;
                    // A/B returns Ok only after each retained runtime, native
                    // host and disposable persistent store have settled.
                    if result.is_ok() { completed_cleanup.store(true, Ordering::SeqCst); }
                    result
                } else {
                    probe::verify(&handle, &worker_fixture, hold_ui, live_sites, manual_sites,
                        evidence_dir.as_deref(), media_permissions, media_user_ready, &completed_cleanup).await
                };
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
    let mut report = evidence.lock().unwrap().take();
    if code == 0 {
        let report = report.as_mut().expect("smoke evidence");
        report["disposable_evidence_cleanup"] = serde_json::json!(true);
        report["native_cleanup_verified"] = serde_json::json!(true);
        report["evidence_directory_retained"] = serde_json::json!(retained_report_path.is_some());
        let fixture_passed = if compare_identity { report["fixture_checks_passed"] == true } else { report["core_checks_passed"] == true };
        let live_passed = !live_sites || if compare_identity { report["live_checks_passed"] == true }
            else { report["live_sites"]["entry_checks_passed"] == true };
        if !fixture_passed || !live_passed { code = 2; }
        if let Some(path) = retained_report_path {
            if std::fs::write(path, serde_json::to_vec_pretty(&report).expect("report serialization")).is_err() {
                eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL retained report update failed");
                code = 1;
            }
        }
    }
    // The authenticated launcher receipt must describe the final process code,
    // including an incomplete website/A-B run after successful native cleanup.
    if let Some(bootstrap) = &mut bootstrap {
        if let Err(error) = bootstrap.report_exit(code, cleanup_verified) {
            eprintln!("MACOS_BROWSER_COMPATIBILITY_SMOKE_FAIL application shutdown receipt failed: {error}");
            code = 1;
        }
    }
    if code == 0 { println!("MACOS_BROWSER_COMPATIBILITY_SMOKE_PASS {}", report.expect("smoke evidence")); }
    else if code == 2 { println!("MACOS_BROWSER_COMPATIBILITY_SMOKE_INCOMPLETE {}", report.expect("smoke evidence")); }
    std::process::exit(code);
}

#[cfg(not(target_os = "macos"))]
fn main() { eprintln!("macos_browser_compatibility_smoke requires macOS 14 or newer"); std::process::exit(1); }
