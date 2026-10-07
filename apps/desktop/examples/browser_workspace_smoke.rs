//! Real child-WebView2 smoke. Run: cargo run -p nomifun-desktop --example browser_workspace_smoke
//! This uses a disposable local fixture. --agent-only additionally starts an
//! isolated application backend with a local scripted model endpoint.
//! --html-drag-only selects the same-document native drag regression. The
//! default matrix also proves that a drag across native frame sessions is
//! rejected before browser input begins.

#[cfg(windows)]
#[path = "../src/browser_surface/automation.rs"]
mod automation;
#[cfg(windows)]
#[path = "support/browser_picker_selection.rs"]
mod picker_selection;
#[cfg(windows)]
#[path = "../src/browser_surface/host.rs"]
mod host;
#[path = "../src/browser_surface/security.rs"]
mod security;
#[cfg(windows)]
#[path = "../src/native_api_plugins.rs"]
mod native_api_plugins;

#[cfg(windows)]
#[path = "../src/browser_surface/windows.rs"]
mod windows;
#[cfg(windows)]
use windows as native;
#[cfg(windows)]
#[path = "support/browser_presentation.rs"]
mod presentation;
#[cfg(windows)]
#[path = "support/browser_resource_fixture.rs"]
mod browser_resource_fixture;
#[cfg(windows)]
#[path = "support/browser_agent_turn.rs"]
mod agent_turn;
#[cfg(windows)]
#[path = "support/browser_live_agent.rs"]
mod live_agent;
#[cfg(windows)]
#[path = "support/browser_crash.rs"]
mod crash_recovery;
#[cfg(windows)]
#[path = "support/browser_diagnostics.rs"]
mod diagnostic_checks;
#[cfg(windows)]
#[path = "support/browser_close_all.rs"]
mod close_all_checks;
#[cfg(windows)]
#[path = "support/browser_site_data.rs"]
mod site_data_probe;
#[cfg(windows)]
#[path = "support/browser_permissions.rs"]
mod permission_checks;
#[cfg(windows)]
#[path = "support/browser_upload_frames.rs"]
mod upload_frames;
#[cfg(windows)]
#[path = "support/browser_user_files.rs"]
mod user_files;
#[cfg(windows)]
#[path = "support/browser_user_downloads.rs"]
mod user_downloads;
#[cfg(windows)]
#[path = "support/browser_external.rs"]
mod external_browser;
#[cfg(windows)]
#[path = "support/browser_evaluation.rs"]
mod browser_evaluation;
#[cfg(windows)]
#[cfg(windows)]
#[path = "support/browser_dialog_probe.rs"]
mod dialog_probe;
#[cfg(windows)]
#[path = "support/browser_dialog_runtime.rs"]
mod dialog_runtime;
#[cfg(windows)]
#[path = "support/browser_dialog_navigation.rs"]
mod dialog_navigation;
#[cfg(windows)]
#[path = "support/browser_dialog_close.rs"]
mod dialog_close;

#[cfg(windows)]
static DELAYED_NAVIGATION_STARTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(windows)]
static EXTERNAL_PAGE_REQUESTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

#[cfg(windows)]
fn main() {
    if let Some(code)=windows::user_file_picker::helper_entry() {
        std::process::exit(if code==std::process::ExitCode::SUCCESS {0}else{1});
    }
    let live_agent_only=std::env::args().any(|arg|arg=="--live-agent-only");
    let live_credential=if live_agent_only {
        match live_agent::credentials() {Ok(key)=>Some(key),Err(code)=>{eprintln!("BROWSER_WORKSPACE_SMOKE_FAIL {code}");std::process::exit(1);}}
    }else{None};
    let _=tracing_subscriber::fmt().with_max_level(tracing::Level::WARN)
        .with_writer(std::io::stderr).without_time().try_init();
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };
    use tauri::Manager;

    let listener = TcpListener::bind("127.0.0.1:0").expect("fixture server");
    let address = listener.local_addr().unwrap();
    let profile = tempfile::tempdir().expect("isolated native fixture profile");
    let profile_path = profile.path().to_path_buf();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            // Chromium can keep speculative connections idle. One such socket
            // must not block every subsequent iframe request in this fixture.
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut request = [0u8; 4096];
                if stream.read(&mut request).is_err() {
                    return;
                }
                let request = String::from_utf8_lossy(&request);
                let path = request
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .split('?')
                    .next()
                    .unwrap_or("");
                if path == "/site-data-probe-worker.js" {
                    let body = "self.addEventListener('install',()=>self.skipWaiting());self.addEventListener('activate',event=>event.waitUntil(self.clients.claim()));";
                    let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).as_bytes());
                    return;
                }
                if path == "/download-redirect" {
                    let _ = stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: /download-file\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    return;
                }
                if path.starts_with("/diagnostic-failure/") {
                    let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    return;
                }
                if path == "/download-interrupted" {
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"interrupted.txt\"\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n");
                    let _ = stream.write_all(&[b'x'; 65536]);
                    let _ = stream.flush();
                    return; // Real truncated HTTP body; no fake native state.
                }
                if path == "/download-slow" {
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(180)));
                    let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=\"native-pending.txt\"\r\nContent-Length: 1048576\r\nConnection: close\r\n\r\n");
                    let _ = stream.write_all(&[b'x'; 65536]);
                    let _ = stream.flush();
                    // Keep the response in flight until the browser cancels it.
                    let _ = stream.read(&mut [0u8; 1]);
                    return;
                }
                if path == "/download-file" {
                    let body = "Native WebView download 中文\n";
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: attachment; filename=\"native-download.txt\"\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                    let _ = stream.write_all(response.as_bytes());
                    return;
                }
                if matches!(path, "/download-disguised" | "/download-executable" | "/download-oversize") {
                    let (name, body, size) = match path {
                        "/download-executable" => ("unsafe.exe", "harmless fixture", 16u64),
                        "/download-oversize" => ("large.txt", "x", 536870913u64),
                        _ => ("disguised.txt", "MZ fake executable fixture", 26u64),
                    };
                    let size = if path == "/download-oversize" { size } else { body.len() as u64 };
                    let _ = stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n{body}").as_bytes());
                    return;
                }
                if path == "/slow-navigation" {
                    DELAYED_NAVIGATION_STARTED.store(true, std::sync::atomic::Ordering::SeqCst);
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
                let body = if path == "/external-browser-fixture" {
                    EXTERNAL_PAGE_REQUESTS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    "<!doctype html><meta charset=utf-8><title>NomiFun external browser check</title><h1>NomiFun browser check</h1><p>This temporary local page verifies the system-browser handoff. It can be closed.</p>".to_owned()
                } else if path == "/evaluation-fixture" {
                    browser_evaluation::HTML.to_owned()
                } else if path == "/evaluation-start" {
                    browser_evaluation::EXECUTION_STARTED.store(true, std::sync::atomic::Ordering::SeqCst);
                    "started".to_owned()
                } else if path == "/popup-source" {
                    include_str!("fixtures/browser_popup.html").to_owned()
                } else if path == "/diagnostic-scopes" {
                    include_str!("fixtures/browser_diagnostics.html").replace("__PORT__", &address.port().to_string())
                } else if path == "/diagnostic-child" {
                    include_str!("fixtures/browser_diagnostic_child.html").replace("__PORT__", &address.port().to_string())
                } else if path == "/popup-child" {
                    include_str!("fixtures/browser_popup_child.html").to_owned()
                } else if path == "/html-drag-frames" {
                    include_str!("fixtures/browser_drag_frames.html")
                        .replace("__PORT__", &address.port().to_string())
                } else if matches!(path, "/html-drag-source" | "/html-drag-target") {
                    let styles = if path == "/html-drag-source" {
                        "#target,#capture,#captureTarget {display:none}"
                    } else {
                        "#source,#capture,#captureTarget {display:none} #target {left:50px}"
                    };
                    include_str!("fixtures/browser_drag.html")
                        .replace("</style>", &format!("{styles}</style>"))
                } else if path == "/html-drag" {
                    include_str!("fixtures/browser_drag.html").to_owned()
                } else if path == "/upload-frames" {
                    include_str!("fixtures/browser_upload_frames.html").replace("__PORT__",&address.port().to_string())
                } else if path == "/dialog-initial" {
                    include_str!("fixtures/browser_dialog_initial.html").to_owned()
                } else if path == "/dialog-popup" {
                    include_str!("fixtures/browser_dialog_popup.html").to_owned()
                } else if path == "/user-files" {
                    include_str!("fixtures/browser_user_files.html").replace("__PORT__",&address.port().to_string())
                } else if path == "/user-downloads" {
                    include_str!("fixtures/browser_user_downloads.html").to_owned()
                } else if matches!(path,"/upload-child-cross"|"/upload-child-same") {
                    include_str!("fixtures/browser_upload_child.html").replace("__LEVEL__",if path=="/upload-child-cross" {"Cross"} else {"Same"})
                } else if path == "/frame-sessions" {
                    include_str!("fixtures/browser_frames.html")
                        .replace("__PORT__", &address.port().to_string())
                } else if matches!(path, "/frame-child" | "/frame-nested" | "/frame-same" | "/frame-same-nested") {
                    let (level, nested) = match path {
                        "/frame-child" => (
                            "Cross-site",
                            format!(
                                r#"<iframe src="http://127.0.0.1:{}/frame-nested"></iframe>"#,
                                address.port()
                            ),
                        ),
                        "/frame-nested" => ("Nested", String::new()),
                        "/frame-same-nested" => ("Same-nested", String::new()),
                        _ => ("Same-process", String::new()),
                    };
                    include_str!("fixtures/browser_frame_content.html")
                        .replace("__LEVEL__", level)
                        .replace("__NESTED__", &nested)
                } else {
                    include_str!("fixtures/browser_workspace.html").to_owned()
                };
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            });
        }
    });
    let success = Arc::new(AtomicBool::new(false));
    let completed = success.clone();
    let evidence = Arc::new(Mutex::new(None));
    let completed_evidence = evidence.clone();
    let verify_failure_exit = std::env::args().any(|arg| arg == "--verify-failure-exit");
    let html_drag_only = std::env::args().any(|arg| arg == "--html-drag-only");
    let popup_only = std::env::args().any(|arg| arg == "--popup-only");
    let diagnostics_only = std::env::args().any(|arg| arg == "--diagnostics-only");
    let close_all_only = std::env::args().any(|arg| arg == "--close-all-only");
    let dialog_close_only = std::env::args().any(|arg| arg == "--dialog-close-only");
    let frame_input_only = std::env::args().any(|arg| arg == "--frame-input-only");
    let upload_frames_only = std::env::args().any(|arg| arg == "--upload-frames-only");
    let managed_popup_only = std::env::args().any(|arg| arg == "--managed-popup-only");
    let presentation_only = std::env::args().any(|arg| arg == "--presentation-only");
    let frame_drag_only = std::env::args().any(|arg| arg == "--frame-drag-only");
    let crash_only = std::env::args().any(|arg| arg == "--crash-only");
    let permissions_only = std::env::args().any(|arg| arg == "--permissions-only");
    let permission_timeout_only = std::env::args().any(|arg| arg == "--permission-timeout-only");
    let runtime_locks_only = std::env::args().any(|arg| arg == "--runtime-locks-only");
    let workspace_only = std::env::args().any(|arg| arg == "--workspace-only");
    let agent_only = std::env::args().any(|arg| arg == "--agent-only");
    let picker_only = std::env::args().any(|arg| arg == "--picker-only");
    let user_files_only = std::env::args().any(|arg| arg == "--user-files-only");
    let user_file_selection_only = std::env::args().any(|arg| arg == "--user-file-selection-only");
    let user_file_filter_only = std::env::args().any(|arg| arg == "--user-file-filter-only");
    let dialog_probe_only = std::env::args().any(|arg| arg == "--dialog-probe-only");
    let picker_selection_only = std::env::args().any(|arg| arg == "--picker-selection-only");
    let save_picker_selection_only = std::env::args().any(|arg| arg == "--save-picker-selection-only");
    let user_downloads_only = std::env::args().any(|arg| arg == "--user-downloads-only");
    let external_browser_only = std::env::args().any(|arg| arg == "--external-browser-only");
    let downloads_folder_only = std::env::args().any(|arg| arg == "--downloads-folder-only");
    let evaluation_only = std::env::args().any(|arg| arg == "--evaluate-only");
    let site_data_probe_only = std::env::args().any(|arg| arg == "--site-data-probe-only");
    let user_download_selection_only = std::env::args().any(|arg| arg == "--user-download-selection-only");
    let user_download_cancel_active_only = std::env::args().any(|arg| arg == "--user-download-cancel-active-only");
    let app = tauri::Builder::default()
        .plugin(native_api_plugins::dialog())
        .plugin(native_api_plugins::notification())
        .invoke_handler(security::app_commands_only(|invoke| {
            invoke
                .resolver
                .resolve("should never be callable from browser fixture");
            true
        }))
        .setup(move |app| {
            // The adapter must retain the real plugin's native service setup.
            let _ = tauri_plugin_dialog::DialogExt::dialog(app.handle());
            let _ = tauri_plugin_notification::NotificationExt::notification(app.handle());
            tauri::window::WindowBuilder::new(app, "main")
                .title("NomiFun Browser Workspace — native input smoke")
                .inner_size(1100.0, 720.0)
                .min_inner_size(880.0, 600.0)
                .build()?;
            let handle = app.handle().clone();
            let watchdog = handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(if live_agent_only {360} else if user_file_selection_only || user_file_filter_only || user_download_selection_only || user_download_cancel_active_only {330} else if save_picker_selection_only {240} else if picker_selection_only { 210 } else if agent_only { 150 } else { 60 })).await;
                eprintln!("BROWSER_WORKSPACE_SMOKE_FAIL native creation or shutdown timed out");
                watchdog.exit(1);
            });
            tauri::async_runtime::spawn(async move {
                let worker_handle = handle.clone();
                // Under the existing exact disposable fixture cleanup owner;
                // these Profiles are persistent, unlike the baseline view.
                let site_data_root = profile_path.join("site-data-conformance");
                // add_child waits for UI dispatch, so create it off the UI thread.
                let child = tauri::async_runtime::spawn_blocking(move || {
                    worker_handle.get_window("main").unwrap().add_child(
                        tauri::webview::WebviewBuilder::new(
                            "browser-smoke",
                            tauri::WebviewUrl::External(
                                format!("http://{address}/").parse().unwrap(),
                            ),
                        )
                        .data_directory(profile_path)
                        .incognito(true)
                        // This fixture must exercise real process-separated frames,
                        // not infer OOPIF support from an ordinary iframe alone.
                        .additional_browser_args("--site-per-process")
                        .disable_drag_drop_handler(),
                        tauri::LogicalPosition::new(20.0, 60.0),
                        tauri::LogicalSize::new(1060.0, 640.0),
                    )
                })
                .await;
                let result = match child {
                    Ok(Ok(view)) => {
                        let result = tokio::time::timeout(std::time::Duration::from_secs(if live_agent_only {330} else if user_file_selection_only || user_file_filter_only || user_download_selection_only || user_download_cancel_active_only {300} else if save_picker_selection_only {210} else if picker_selection_only { 180 } else if agent_only { 120 } else { 45 }), async {
                            if let Some(key)=live_credential {
                                view.hide().map_err(|_|"LIVE_INITIAL_VIEW_FAILED")?;
                                let live_handle=handle.clone();
                                let evidence=tauri::async_runtime::spawn_blocking(move ||live_agent::run(live_handle,key)).await.map_err(|_|"LIVE_WORKER_FAILED")??;
                                return Ok(serde_json::json!({"scope":"live-agent-only","agent":evidence}));
                            }
                            if picker_selection_only {
                                view.hide().map_err(|error|error.to_string())?;
                                return picker_selection::verify().await;
                            }
                            if agent_only {
                                view.hide().map_err(|error|error.to_string())?;
                                let agent_handle=handle.clone();
                                let agent=tauri::async_runtime::spawn_blocking(move || {
                                    agent_turn::run(agent_handle)
                                }).await.map_err(|error|error.to_string())??;
                                return Ok(serde_json::json!({"scope":"agent-only","agent":agent}));
                            }
                            if frame_input_only {
                                let input=verify(&view).await?;
                                let frames=verify_frame_input(&view,&format!("http://{address}/frame-sessions")).await?;
                                return Ok(serde_json::json!({"scope":"frame-input-only","input":input,"frames":frames}));
                            }
                            if dialog_probe_only {
                                let dialogs=dialog_probe::verify(&view).await?;
                                let runtime=dialog_runtime::verify(&handle,&format!("http://{address}/")).await?;
                                let navigation=dialog_navigation::verify(&handle,&format!("http://{address}/")).await?;
                                let close=dialog_close::verify(&handle,&format!("http://{address}/")).await?;
                                return Ok(serde_json::json!({"scope":"dialog-probe-only","dialogs":dialogs,"runtime":runtime,"navigation":navigation,"close":close}));
                            }
                            if dialog_close_only {
                                let close=dialog_close::verify(&handle,&format!("http://{address}/")).await?;
                                return Ok(serde_json::json!({"scope":"dialog-close-only","close":close}));
                            }
                            if upload_frames_only {
                                let files=upload_frames::verify(&view,&format!("http://{address}/upload-frames")).await?;
                                return Ok(serde_json::json!({"scope":"upload-frames-only","files":files}));
                            }
                            if save_picker_selection_only {
                                view.hide().map_err(|error|error.to_string())?;
                                let selected=picker_selection::verify_save().await?;
                                return Ok(serde_json::json!({"scope":"save-picker-selection-only","selected":selected}));
                            }
                            if external_browser_only {
                                let result=external_browser::verify(&handle,&format!("http://{address}/external-browser-fixture?nomifun-check={}#handoff",uuid::Uuid::now_v7())).await?;
                                return Ok(serde_json::json!({"scope":"external-browser","handoff":result}));
                            }
                            if downloads_folder_only {
                                let result = external_browser::verify_downloads(&handle, &format!("http://{address}/")).await?;
                                return Ok(serde_json::json!({"scope":"downloads-folder","handoff":result}));
                            }
                            if evaluation_only {
                                let result = browser_evaluation::verify(&handle, &format!("http://{address}/evaluation-fixture")).await?;
                                return Ok(serde_json::json!({"scope":"developer-evaluation","evaluation":result}));
                            }
                            if site_data_probe_only {
                                let result=site_data_probe::verify(&handle,&format!("http://{address}/"),&site_data_root).await?;
                                return Ok(serde_json::json!({"scope":"site-data-probe-only","site_data":result}));
                            }
                            if user_downloads_only || user_download_selection_only || user_download_cancel_active_only {
                                view.hide().map_err(|e|e.to_string())?;
                                let check=if user_download_cancel_active_only {user_downloads::Check::ActiveCancel} else if user_download_selection_only {user_downloads::Check::Save} else {user_downloads::Check::Lifecycle};
                                let downloads=user_downloads::verify(&handle,&format!("http://{address}/user-downloads"),check).await?;
                                return Ok(serde_json::json!({"scope":"user-downloads","downloads":downloads}));
                            }
                            if picker_only {
                                view.hide().map_err(|error|error.to_string())?;
                                let root=tempfile::tempdir().map_err(|error|error.to_string())?;
                                let picker=windows::user_file_picker::NativeFilePicker::start(windows::user_file_picker::Options {
                                    title:"NomiFun isolated picker cancellation test".into(),initial_directory:root.path().into(),mode:windows::user_file_picker::PickerMode::Open { multiple:true },extensions:vec!["txt".into()],
                                })?;
                                tokio::time::timeout(std::time::Duration::from_secs(8),picker.opened()).await.map_err(|_|"Native picker did not show")??;
                                let other=windows::user_file_picker::NativeFilePicker::start(windows::user_file_picker::Options {
                                    title:"NomiFun independent picker test".into(),initial_directory:root.path().into(),mode:windows::user_file_picker::PickerMode::Open { multiple:false },extensions:vec![],
                                })?;
                                tokio::time::timeout(std::time::Duration::from_secs(8),other.opened()).await.map_err(|_|"Second native picker did not show")??;
                                picker.close().await?;
                                if picker.finished().await?.is_some() {return Err("Cancelled picker returned file paths".into());}
                                picker.close().await?;
                                if tokio::time::timeout(std::time::Duration::from_millis(150),other.finished()).await.is_ok() {return Err("Cancelling one picker closed the independent picker".into());}
                                let receipt=other.exit_receipt().await?;
                                drop(other);
                                tokio::time::timeout(std::time::Duration::from_secs(5),receipt.wait()).await.map_err(|_|"Dropped picker did not close")?.map_err(|error|error.to_string())?;
                                let early=windows::user_file_picker::NativeFilePicker::start(windows::user_file_picker::Options {
                                    title:"NomiFun early picker cancellation".into(),initial_directory:root.path().into(),mode:windows::user_file_picker::PickerMode::Open { multiple:false },extensions:vec![],
                                })?;
                                early.close().await?;
                                if early.finished().await?.is_some() {return Err("Early cancellation returned files".into());}
                                let save=windows::user_file_picker::NativeFilePicker::start(windows::user_file_picker::Options {
                                    title:"NomiFun save download cancellation test".into(),initial_directory:root.path().into(),
                                    mode:windows::user_file_picker::PickerMode::Save { filename:"下载 空格.txt".into() },extensions:vec!["txt".into()],
                                })?;
                                tokio::time::timeout(std::time::Duration::from_secs(8),save.opened()).await.map_err(|_|"Native save dialog did not show")??;
                                let save_exit=save.exit_receipt().await?;
                                save.close().await?;
                                if save.finished().await?.is_some() {return Err("Cancelled save returned a path".into());}
                                tokio::time::timeout(std::time::Duration::from_secs(5),save_exit.wait()).await.map_err(|_|"Save dialog process did not exit")?.map_err(|error|error.to_string())?;
                                if root.path().join("下载 空格.txt").exists() {return Err("Cancelled save created a download file".into());}
                                return Ok(serde_json::json!({"scope":"picker-only","real_native_window":true,"cancelled_process_tree":true,"independent_pickers":true,"drop_closes_process":true,"early_cancel":true,"save_dialog":true,"save_cancelled_without_file":true,"no_selected_files":true}));
                            }
                            if user_files_only || user_file_selection_only || user_file_filter_only {
                                view.hide().map_err(|error|error.to_string())?;
                                let check=if user_file_filter_only {user_files::Check::Filter} else if user_file_selection_only {user_files::Check::Selection} else {user_files::Check::Lifecycle};
                                let files=user_files::verify(&handle,&format!("http://{address}/user-files"),check).await?;
                                return Ok(serde_json::json!({"scope":if user_file_filter_only {"user-file-filter-only"} else if user_file_selection_only {"user-file-selection-only"} else {"user-files-only"},"files":files}));
                            }
                            if workspace_only {
                                view.hide().map_err(|error|error.to_string())?;
                                let workspace=verify_workspace(&handle,&format!("http://{address}/")).await?;
                                return Ok(serde_json::json!({"scope":"workspace-only","workspace":workspace}));
                            }
                            if permissions_only || permission_timeout_only {
                                view.hide().map_err(|error|error.to_string())?;
                                let permissions=permission_checks::verify(&handle,&format!("http://{address}/popup-source"),permission_timeout_only).await?;
                                return Ok(serde_json::json!({"scope":"permissions","permissions":permissions}));
                            }
                            if crash_only {
                                view.hide().map_err(|error|error.to_string())?;
                                let crash=crash_recovery::verify(&handle,&format!("http://{address}/popup-source")).await?;
                                return Ok(serde_json::json!({"scope":"crash-only","crash":crash}));
                            }
                            if managed_popup_only {
                                view.hide().map_err(|error|error.to_string())?;
                                let popup=verify_managed_popup(&handle,&format!("http://{address}/popup-source")).await?;
                                return Ok(serde_json::json!({"scope":"managed-popup-only","managed_popup":popup}));
                            }
                            if diagnostics_only {
                                let diagnostics=diagnostic_checks::verify(&view,&format!("http://{address}/diagnostic-scopes")).await?;
                                return Ok(serde_json::json!({"scope":"diagnostics-only","diagnostics":diagnostics}));
                            }
                            if close_all_only {
                                let closed=close_all_checks::verify(&handle,&format!("http://{address}/popup-source")).await?;
                                return Ok(serde_json::json!({"scope":"close-all-only","closed":closed}));
                            }
                            if presentation_only {
                                view.hide().map_err(|error|error.to_string())?;
                                let presentation=presentation::verify(&handle,&format!("http://{address}/popup-source")).await?;
                                return Ok(serde_json::json!({"scope":"presentation-only","presentation":presentation}));
                            }
                            if frame_drag_only {
                                let unsupported=verify_frame_drag_unsupported(&view,&format!("http://{address}/html-drag-frames")).await?;
                                return Ok(serde_json::json!({"scope":"frame-drag-only","unsupported":unsupported}));
                            }
                            if runtime_locks_only {
                                view.hide().map_err(|error|error.to_string())?;
                                let locks=verify_runtime_locks(&handle,&format!("http://{address}/popup-source")).await?;
                                return Ok(serde_json::json!({"scope":"runtime-locks-only","runtime_locks":locks}));
                            }
                            if popup_only {
                                let popup=verify_popups(&view,&format!("http://{address}/popup-source")).await?;
                                return Ok(serde_json::json!({"scope":"popup-only","popup":popup}));
                            }
                            if html_drag_only {
                                let html_drag=verify_html_drag(&view,&format!("http://{address}/html-drag")).await?;
                                return Ok(serde_json::json!({"scope":"html-drag-only","html_drag":html_drag}));
                            }
                            let input_evidence = verify(&view).await?;
                            let frame_drag_unsupported = verify_frame_drag_unsupported(&view, &format!("http://{address}/html-drag-frames")).await?;
                            Ok(serde_json::json!({"scope":"default","input_evidence":input_evidence,"frame_drag_unsupported":frame_drag_unsupported}))
                        }).await;
                        let closed = if handle.get_webview(view.label()).is_some() { verify_native_close(&view).await } else { Ok(()) };
                        match result {
                            Ok(result) => result.and_then(|evidence| closed.map(|()| evidence)),
                            Err(_) => Err("Native smoke timed out.".to_owned()),
                        }
                    }
                    _ => Err("Native child creation failed.".to_owned()),
                };
                let result = if verify_failure_exit && result.is_ok() {
                    Err("Intentional failure to verify nonzero exit and cleanup.".to_owned())
                } else { result };
                match result {
                    Ok(evidence) => {
                        *completed_evidence.lock().unwrap() = Some(evidence);
                        completed.store(true, Ordering::SeqCst);
                    }
                    Err(error) => eprintln!("BROWSER_WORKSPACE_SMOKE_FAIL {error}"),
                }
                handle.exit(if completed.load(Ordering::SeqCst) { 0 } else { 1 });
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("native smoke application");
    // run() does not return to the cleanup/assertions below on every platform.
    // Keep the exit result and prove fixture profile cleanup before exiting.
    let exit_code = app.run_return(|_, _| {});
    if let Err(error) = cleanup_fixture_profile(&profile) {
        eprintln!("BROWSER_WORKSPACE_SMOKE_FAIL native fixture profile cleanup: {error}");
        std::process::exit(1);
    }
    if exit_code != 0 || !success.load(Ordering::SeqCst) {
        std::process::exit(1);
    }
    let mut evidence = evidence
        .lock()
        .unwrap()
        .take()
        .expect("native smoke evidence");
    evidence["native_fixture_profile_cleanup"] = serde_json::json!(true);
    println!("BROWSER_WORKSPACE_SMOKE_PASS {evidence}");
}

#[cfg(windows)]
fn cleanup_fixture_profile(profile: &tempfile::TempDir) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        // The exact disposable fixture directory remains owned until removal.
        // WebView2 file handles can outlive controller Close briefly; never kill
        // a shared browser process or treat a queued close as deletion proof.
        match std::fs::remove_dir_all(profile.path()) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) if std::time::Instant::now() >= deadline => return Err(error.to_string()),
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(25)),
        }
    }
}

#[cfg(windows)]
async fn verify_native_close(view: &tauri::Webview) -> Result<(), String> {
    let events = windows::frames::FrameSessions::connect(view).await;
    windows::close_native_view(view).await?;
    let mut events = events?;
    if events.trees().await.is_ok() {
        return Err("Closed native controller retained usable frame subscriptions.".into());
    }
    Ok(())
}

#[cfg(windows)]
async fn popup_interaction<T, F, Fut>(
    driver: &mut automation::TabAutomation,
    view: &tauri::Webview,
    subscription: &mut windows::popup::PopupSubscription,
    name: &str,
    may_destroy_opener: bool,
    process: F,
) -> Result<T, String>
where
    F: FnOnce(windows::popup::PopupRequest) -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    let element = frame_element(driver, view, name).await?;
    let cancel = tokio_util::sync::CancellationToken::new();
    let process = async {
        let request = tokio::time::timeout(std::time::Duration::from_secs(3), subscription.next())
            .await
            .map_err(|_| "Native popup request was not delivered")?
            .ok_or("Popup subscription closed")?;
        process(request).await
    };
    // WebView2 can defer completion of the triggering input until the new
    // window request settles. Its consumer must run alongside that input.
    let (input, result) = tokio::join!(
        driver.act(
            view,
            nomifun_browser_platform::runtime::BrowserAction::click(element),
            &cancel
        ),
        process
    );
    let result = result?;
    if !may_destroy_opener {
        input.map_err(|error| error.to_string())?;
    }
    Ok(result)
}

#[cfg(windows)]
async fn verify_popups(view: &tauri::Webview, url: &str) -> Result<serde_json::Value, String> {
    use nomifun_browser_platform::runtime::BrowserAction;
    use serde_json::json;
    let operation = std::sync::Arc::new(std::sync::Mutex::new(
        tokio_util::sync::CancellationToken::new(),
    ));
    let operation_for_request = operation.clone();
    let opener_label = view.label().to_owned();
    let closed = tokio_util::sync::CancellationToken::new();
    let mut subscription = windows::popup::PopupSubscription::listen_guarded(
        view,
        std::sync::Arc::new(move || {
            Some(windows::popup::PopupAdmission {
                download: None,
                opener: nomifun_browser_platform::runtime::BrowserTabTarget {
                    tab_id: opener_label.clone(),
                    runtime_generation: 1,
                    document_generation: 1,
                },
                cancel: operation_for_request.lock().unwrap().clone(),
                closed: closed.clone(),
            })
        }),
    )
    .await?;
    if windows::popup::PopupSubscription::listen(view)
        .await
        .is_ok()
    {
        return Err("Duplicate popup owner was accepted".into());
    }
    windows::protocol_call(view, "Page.navigate", json!({"url":url})).await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if evaluate(
            view,
            "!!window.popupNonce && document.readyState==='complete'",
        )
        .await?
            == true
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Popup fixture did not load".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    evaluate(view, "setTimeout(()=>window.open('/popup-child'),0);true").await?;
    if tokio::time::timeout(std::time::Duration::from_millis(150), subscription.next())
        .await
        .is_ok()
        || subscription.pending_count().await? != 0
    {
        return Err("A non-user popup was admitted".into());
    }
    let nonce = evaluate(view, "window.popupNonce").await?;
    let mut driver = automation::TabAutomation::default();
    windows::set_user_input_enabled(view, false).await?;
    let child = popup_interaction(
        &mut driver,
        view,
        &mut subscription,
        "Open real popup",
        false,
        |request| async move {
            if request.opener_label != view.label() || !request.url.ends_with("/popup-child") {
                return Err("Popup opener/URL association was wrong".into());
            }
            let (loaded_tx, mut loaded_rx) = tokio::sync::mpsc::channel(8);
            let child = request
                .create_child(move |builder| {
                    builder.on_page_load(move |_, payload| {
                        if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
                            let _ = loaded_tx.try_send(payload.url().to_string());
                        }
                    })
                })
                .await
                .map_err(|error| format!("Creating URL popup: {error}"))?;
            windows::set_user_input_enabled(&child, false).await?;
            request
                .complete(&child)
                .await
                .map_err(|error| format!("Binding URL popup: {error}"))?;
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while let Some(url) = loaded_rx.recv().await {
                    if url.ends_with("/popup-child") {
                        return Ok(());
                    }
                }
                Err("Native popup loading channel closed")
            })
            .await
            .map_err(|_| "Bound native popup did not load")??;
            Ok(child)
        },
    )
    .await?;
    let proof = evaluate(&child, "window.popupProof").await?;
    if proof["hasOpener"] != true
        || proof["ownTop"] != true
        || proof["nonce"] != nonce
        || !proof["cookie"]
            .as_str()
            .is_some_and(|cookie| cookie.contains("nomi_popup=shared-profile"))
    {
        return Err(format!("Native popup lost opener or profile: {proof}"));
    }
    let delivered=evaluate(view,r#"new Promise(resolve=>{
        const ready=()=>popupMessages.some(message=>message.sameProxy && message.data.nomiPopup);
        if(ready()){resolve(true);return}
        const finish=value=>{clearTimeout(timer);removeEventListener('message',listener);resolve(value)};
        const listener=()=>{if(ready())finish(true)};
        const timer=setTimeout(()=>finish(false),1000);addEventListener('message',listener);
    })"#).await?;
    if delivered != true {
        return Err("Popup postMessage did not come from the original WindowProxy".into());
    }
    child
        .set_position(tauri::LogicalPosition::new(20.0, 60.0))
        .map_err(|error| error.to_string())?;
    child
        .set_size(tauri::LogicalSize::new(850.0, 580.0))
        .map_err(|error| error.to_string())?;
    child.show().map_err(|error| error.to_string())?;
    let mut child_driver = automation::TabAutomation::default();
    let reply = frame_element(&mut child_driver, &child, "Reply from popup").await?;
    child_driver
        .act(
            &child,
            BrowserAction::click(reply),
            &tokio_util::sync::CancellationToken::new(),
        )
        .await
        .map_err(|error| format!("Native popup interaction failed: {error}"))?;
    let interacted=evaluate(view,r#"new Promise(resolve=>{
        const ready=()=>popupMessages.some(message=>message.sameProxy && message.data.interaction && message.data.trusted);
        if(ready()){resolve(true);return}
        const finish=value=>{clearTimeout(timer);removeEventListener('message',listener);resolve(value)};
        const listener=()=>{if(ready())finish(true)};
        const timer=setTimeout(()=>finish(false),1000);addEventListener('message',listener);
    })"#).await?;
    if interacted != true {
        return Err("Displayed popup did not receive trusted native input".into());
    }
    child_driver
        .release(&child)
        .await
        .map_err(|error| error.to_string())?;
    if evaluate(
        &child,
        "window.__TAURI_INTERNALS__.invoke('smoke_forbidden').then(()=>false,()=>true)",
    )
    .await?
        != true
    {
        return Err("Popup child acquired app IPC".into());
    }
    windows::close_native_view(&child).await?;
    let blank = popup_interaction(
        &mut driver,
        view,
        &mut subscription,
        "Open writable blank popup",
        false,
        |request| async move {
            if request.url != "about:blank" {
                return Err("Blank popup URL changed".into());
            }
            let blank = request.create_child(|builder| builder).await?;
            windows::set_user_input_enabled(&blank, false).await?;
            request.complete(&blank).await?;
            Ok(blank)
        },
    )
    .await?;
    let written = evaluate(
        &blank,
        "({text:document.getElementById('written')?.textContent,nonce:opener?.popupNonce})",
    )
    .await?;
    if written["text"] != "Written by opener" || written["nonce"] != nonce {
        return Err(format!("Deferred popup lost document.write: {written}"));
    }
    windows::close_native_view(&blank).await?;
    popup_interaction(
        &mut driver,
        view,
        &mut subscription,
        "Open real popup",
        false,
        |request| { let operation=operation.clone(); async move {
            let labels=|| view.window().webviews().into_iter().map(|view|view.label().to_owned()).collect::<std::collections::BTreeSet<_>>();
            let before=labels();
            operation.lock().unwrap().cancel();
            *operation.lock().unwrap() = tokio_util::sync::CancellationToken::new();
            if request.create_child(|builder|builder).await.is_ok() {
                return Err("Stopped popup created a native child under a new operation".into());
            }
            drop(request);
            windows::popup::settle_view(view).await?;
            if labels()!=before { return Err("Stopped popup leaked an unregistered native candidate".into()); }
            Ok(())
        } },
    ).await?;
    popup_interaction(
        &mut driver, view, &mut subscription, "Open real popup", false,
        |request| { let operation=operation.clone(); async move {
            use tauri::Manager;
            let child=request.create_child(|builder|builder).await?;
            windows::set_user_input_enabled(&child,false).await?;
            request.claim_child(&child).await?;
            operation.lock().unwrap().cancel();
            *operation.lock().unwrap()=tokio_util::sync::CancellationToken::new();
            let rejected=request.complete(&child).await.is_err();
            drop(request);
            windows::popup::settle_view(view).await?;
            let retained=view.app_handle().get_webview(child.label()).is_some();
            // This caller now has the same sole close obligation as Runtime.
            windows::close_native_view(&child).await?;
            popup_child_gone(&child).await?;
            if !rejected || !retained { return Err("Cancelled request destroyed its Runtime-owned candidate or bound after Stop".into()); }
            Ok(())
        } },
    ).await?;
    popup_interaction(
        &mut driver,
        view,
        &mut subscription,
        "Open real popup",
        false,
        |request| { let operation=operation.clone(); async move {
            let child = request.create_child(|builder| builder).await?;
            // Satisfy the unrelated input precondition before cancellation so
            // rejection really proves stale operation fencing.
            windows::set_user_input_enabled(&child, false).await?;
            operation.lock().unwrap().cancel();
            *operation.lock().unwrap() = tokio_util::sync::CancellationToken::new();
            if request.complete(&child).await.is_ok() {
                return Err("Cancelled popup adopted a newer operation scope".into());
            }
            drop(request);
            windows::popup::settle_view(view).await?;
            popup_child_gone(&child).await
        } },
    )
    .await?;
    popup_interaction(
        &mut driver,
        view,
        &mut subscription,
        "Open real popup",
        false,
        |request| async move {
            let child = request.create_child(|builder| builder).await?;
            let rejected = request.complete(&child).await.is_err();
            drop(request);
            popup_child_gone(&child).await?;
            if !rejected {
                return Err("Popup binding accepted a child without its input gate".into());
            }
            Ok(())
        },
    )
    .await?;
    popup_interaction(
        &mut driver,
        view,
        &mut subscription,
        "Open real popup",
        false,
        |request| async move {
            let child = request.create_child(|builder| builder).await?;
            let rejected = request.complete(view).await.is_err();
            drop(request);
            popup_child_gone(&child).await?;
            if !rejected {
                return Err("Popup binding replaced an unrelated native view".into());
            }
            Ok(())
        },
    )
    .await?;
    popup_interaction(
        &mut driver,
        view,
        &mut subscription,
        "Open real popup",
        false,
        |request| async move {
            let child = request.create_child(|builder| builder).await?;
            drop(request);
            popup_child_gone(&child).await
        },
    )
    .await?;
    let element = frame_element(&mut driver, view, "Open bounded popup burst").await?;
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut denied = 0usize;
    let mut pending_max = 0usize;
    {
        let burst = driver.act(view, BrowserAction::click(element), &cancel);
        tokio::pin!(burst);
        loop {
            tokio::select! {
                result=&mut burst=>{result.map_err(|error|error.to_string())?;break},
                request=subscription.next()=>{
                    let request=request.ok_or("Popup subscription closed")?;
                    pending_max=pending_max.max(subscription.pending_count().await?);
                    drop(request);denied+=1;
                }
            }
        }
    }
    if denied == 0 || pending_max > 8 || subscription.pending_count().await? != 0 {
        return Err(format!(
            "Popup burst did not settle within its bound: denied={denied},pending={pending_max}"
        ));
    }
    popup_interaction(
        &mut driver,
        view,
        &mut subscription,
        "Open real popup",
        true,
        |request| async move {
            windows::protocol_call(view, "Page.navigate", json!({"url":format!("{url}?next")})).await?;
            if request.create_child(|builder| builder).await.is_ok() {
                return Err("Popup outlived its opener navigation".into());
            }
            Ok(())
        },
    )
    .await?;
    if subscription.pending_count().await? != 0 {
        return Err("Navigation retained popup deferrals".into());
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if evaluate(
            view,
            "!!window.popupNonce && document.readyState==='complete'",
        )
        .await?
            == true
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Opener navigation did not finish".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    popup_interaction(
        &mut driver,
        view,
        &mut subscription,
        "Open real popup",
        true,
        |request| async move {
            windows::close_native_view(view).await?;
            if request.create_child(|builder| builder).await.is_ok() {
                return Err("Closed opener retained its popup request".into());
            }
            Ok(())
        },
    )
    .await?;
    if subscription.pending_count().await? != 0 {
        return Err("Close retained popup deferrals".into());
    }
    Ok(
        json!({"opener_and_profile":proof,"original_window_proxy":true,"writable_blank":written,
        "non_user_denied":true,"pending_max":pending_max,"burst_denied":denied,"dropped_deferrals_settled":true,
        "navigation_invalidates_pending":true,"close_invalidates_pending":true,"child_ipc_denied":true,
        "unguarded_child_rejected":true,"unrelated_child_rejected":true,"unclaimed_child_closed":true,"displayed_child_trusted_input":true,"old_operation_popup_cancelled":true,"cancel_before_child_creation":true,"cancel_after_locked_child_creation":true,"popup_cleanup_barrier":true,"cancel_preserves_runtime_owned_candidate":true}),
    )
}

#[cfg(windows)]
async fn popup_child_gone(view: &tauri::Webview) -> Result<(), String> {
    use tauri::Manager;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while view.app_handle().get_webview(view.label()).is_some() {
        if tokio::time::Instant::now() >= deadline {
            return Err("Unclaimed popup child was not closed".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    Ok(())
}

#[cfg(windows)]
#[path = "support/browser_frame_input.rs"]
mod browser_frame_input;
#[cfg(windows)]
use browser_frame_input::{frame_element, verify_frame_input};

#[cfg(windows)]
async fn set_fixture_browser_zoom(view:&tauri::Webview,zoom:f64)->Result<(),String> {
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result=(||->Result<(),String> {
            unsafe {platform.controller().SetZoomFactor(zoom)}.map_err(|error|error.to_string())?;
            let mut actual=0.;
            unsafe {platform.controller().ZoomFactor(&mut actual)}.map_err(|error|error.to_string())?;
            if (actual-zoom).abs()>1e-9 {return Err("Native zoom readback differs".into());}
            Ok(())
        })();
        let _=tx.send(result);
    }).map_err(|error|error.to_string())?;
    rx.await.map_err(|error|error.to_string())?
}

#[cfg(windows)]
async fn evaluate(view: &tauri::Webview, expression: &str) -> Result<serde_json::Value, String> {
    let result = windows::protocol_call(
        view,
        "Runtime.evaluate",
        serde_json::json!({
            "expression": expression, "returnByValue": true, "awaitPromise": true,
        }),
    )
    .await?;
    if result.get("exceptionDetails").is_some() {
        return Err(format!("Fixture evaluation failed: {result}"));
    }
    Ok(result["result"]["value"].clone())
}

#[cfg(windows)]
async fn verify_html_drag(view: &tauri::Webview, url: &str) -> Result<serde_json::Value, String> {
    use nomifun_browser_platform::{
        run_guard::RunAdmissionError,
        runtime::{BrowserAction, BrowserTabTarget, WorkspaceError},
    };
    use serde_json::json;
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut driver = automation::TabAutomation::default();
    driver
        .initialize_frames(view)
        .await
        .map_err(|error| error.to_string())?;
    windows::protocol_call(view, "Page.navigate", json!({"url":url})).await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if evaluate(
            view,
            "!!window.dragEvidence && document.readyState==='complete'",
        )
        .await?
            == true
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("HTML drag fixture did not load".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    windows::set_user_input_enabled(view, false).await?;
    let target = BrowserTabTarget {
        tab_id: "browser-smoke".into(),
        runtime_generation: 1,
        document_generation: 1,
    };
    for (source, destination) in [
        ("HTML drag source", "HTML drop target"),
        ("Pointer capture source", "Pointer capture target"),
    ] {
        let observation = driver
            .observe(view, target.clone(), &cancel)
            .await
            .map_err(|error| error.to_string())?;
        let reference = |name| {
            observation
                .elements
                .iter()
                .find(|element| element.name == name)
                .map(|element| element.reference.clone())
                .ok_or_else(|| format!("Missing drag fixture element {name}"))
        };
        driver
            .act(
                view,
                BrowserAction::Drag {
                    from: reference(source)?,
                    to: reference(destination)?,
                },
                &cancel,
            )
            .await
            .map_err(|error| format!("Native drag {source}: {error}"))?;
    }
    let result = evaluate(view, "window.dragEvidence").await?;
    if result["drops"] != 1
        || result["text"] != "nomifun-native-drag"
        || result["custom"] != "opaque-value"
    {
        return Err(format!(
            "Native HTML drag did not deliver its browser DataTransfer: {result}"
        ));
    }
    let events = result["events"].as_array().ok_or("Missing drag events")?;
    for kind in ["dragstart", "dragenter", "dragover", "drop", "dragend"] {
        if !events
            .iter()
            .any(|event| event["type"] == kind && event["trusted"] == true)
        {
            return Err(format!("HTML drag omitted trusted {kind}: {result}"));
        }
    }
    if result["capturedMoves"].as_u64().unwrap_or(0) < 2 {
        return Err("HTML drag changes broke pointer capture dragging".into());
    }
    evaluate(
        view,
        "window.dragEvidence={events:[],drops:0,capturedMoves:0};true",
    )
    .await?;
    let observation = driver
        .observe(view, target.clone(), &cancel)
        .await
        .map_err(|error| error.to_string())?;
    let reference = |name| {
        observation
            .elements
            .iter()
            .find(|element| element.name == name)
            .map(|element| element.reference.clone())
            .ok_or_else(|| format!("Missing drag element {name}"))
    };
    let cancelled = tokio_util::sync::CancellationToken::new();
    let stop = async {
        let entered=evaluate(view,r#"new Promise(resolve=>{
            const finish=value=>{clearTimeout(timer);document.removeEventListener('dragover',listener,true);resolve(value)};
            const listener=event=>{if(event.target.id==='target')finish(true)};
            const timer=setTimeout(()=>finish(false),3000);
            document.addEventListener('dragover',listener,{capture:true,passive:true});
        })"#).await;
        cancelled.cancel();
        entered
    };
    let action = BrowserAction::Drag {
        from: reference("HTML drag source")?,
        to: reference("HTML drop target")?,
    };
    let (entered, stopped) = tokio::join!(stop, driver.act(view, action, &cancelled));
    if entered? != true
        || !matches!(
            stopped,
            Err(WorkspaceError::Admission(RunAdmissionError::Cancelled))
        )
    {
        return Err(format!(
            "Drag Stop did not interrupt over a valid target: {stopped:?}"
        ));
    }
    let cancellation = evaluate(view, "window.dragEvidence").await?;
    if cancellation["drops"] != 0
        || !cancellation["events"].as_array().is_some_and(|events| {
            events.iter().any(|event| {
                event["type"] == "dragend"
                    && event["trusted"] == true
                    && event["dropEffect"] == "none"
            })
        })
    {
        return Err(format!(
            "Drag cancellation committed a drop or left drag state active: {cancellation}"
        ));
    }
    // Cleanup must permit another drag on the same document and native view.
    let observation = driver
        .observe(view, target.clone(), &cancel)
        .await
        .map_err(|error| error.to_string())?;
    let reference = |name| {
        observation
            .elements
            .iter()
            .find(|element| element.name == name)
            .map(|element| element.reference.clone())
            .ok_or_else(|| format!("Missing drag element {name}"))
    };
    driver
        .act(
            view,
            BrowserAction::Drag {
                from: reference("HTML drag source")?,
                to: reference("HTML drop target")?,
            },
            &cancel,
        )
        .await
        .map_err(|error| error.to_string())?;
    if evaluate(view, "window.dragEvidence.drops").await? != 1 {
        return Err("Drag did not recover after Stop".into());
    }
    evaluate(view,"window.dragEvidence={events:[],drops:0,replacementDrops:0,capturedMoves:0};window.replaceDragTarget=true;true").await?;
    let observation = driver
        .observe(view, target, &cancel)
        .await
        .map_err(|error| error.to_string())?;
    let reference = |name| {
        observation
            .elements
            .iter()
            .find(|element| element.name == name)
            .map(|element| element.reference.clone())
            .ok_or_else(|| format!("Missing drag element {name}"))
    };
    let replaced = driver
        .act(
            view,
            BrowserAction::Drag {
                from: reference("HTML drag source")?,
                to: reference("HTML drop target")?,
            },
            &cancel,
        )
        .await;
    let replacement = evaluate(view, "window.dragEvidence").await?;
    if !matches!(replaced, Err(WorkspaceError::ActionInterrupted))
        || replacement["replaced"] != true
        || replacement["drops"] != 0
        || replacement["replacementDrops"] != 0
    {
        return Err(format!(
            "Drag committed to a replaced target: {replaced:?}, {replacement}"
        ));
    }
    driver
        .release(view)
        .await
        .map_err(|error| error.to_string())?;
    windows::set_user_input_enabled(view, true).await?;
    Ok(
        json!({"completed":result,"cancelled":cancellation,"same_document_recovery":true,"replacement_rejected":replacement}),
    )
}

#[cfg(windows)]
async fn verify_frame_drag_unsupported(
    view: &tauri::Webview,
    url: &str,
) -> Result<serde_json::Value, String> {
    use nomifun_browser_platform::runtime::{
        BrowserAction, BrowserTabTarget, WorkspaceError,
    };
    use serde_json::json;
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut driver = automation::TabAutomation::default();
    driver
        .initialize_frames(view)
        .await
        .map_err(|error| error.to_string())?;
    windows::protocol_call(view, "Page.navigate", json!({"url":url})).await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if evaluate(
            view,
            "!!window.dragFrameEvidence && document.readyState==='complete'",
        )
        .await?
            == true
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Cross-frame drag fixture did not load".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    windows::set_user_input_enabled(view, false).await?;
    let target = BrowserTabTarget {
        tab_id: "browser-smoke".into(),
        runtime_generation: 1,
        document_generation: 1,
    };
    let observation = driver
        .observe(view, target, &cancel)
        .await
        .map_err(|error| error.to_string())?;
    if observation.unobserved_frames != 0 {
        return Err("Cross-frame drag omitted frame observations".into());
    }
    let reference = |name| {
        observation
            .elements
            .iter()
            .find(|element| element.name == name)
            .map(|element| element.reference.clone())
            .ok_or_else(|| format!("Missing cross-frame drag element {name}"))
    };
    let from = reference("HTML drag source")?;
    let to = reference("HTML drop target")?;
    if from.ref_id.split(':').next() == to.ref_id.split(':').next() {
        return Err("Cross-frame drag fixture reused one frame".into());
    }
    let before = evaluate(view, "window.dragFrameEvidence").await?;
    if before["events"]
        .as_array()
        .is_none_or(|events| !events.is_empty())
        || before["drops"]
            .as_array()
            .is_none_or(|drops| !drops.is_empty())
    {
        return Err(format!(
            "Cross-frame drag fixture was not initially idle: {before}"
        ));
    }
    let action_result = driver
        .act(view, BrowserAction::Drag { from, to }, &cancel)
        .await;
    // Let any incorrectly dispatched browser event cross the iframe message
    // boundary before inspecting the parent evidence. A valid rejection does
    // not need a readiness retry because it returns before mouseDown.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let result = evaluate(view, "window.dragFrameEvidence").await?;
    if !matches!(
        &action_result,
        Err(WorkspaceError::UnsupportedAction)
    ) {
        return Err(format!(
            "Cross-frame drag was not rejected as unsupported before input: {action_result:?}; {result}"
        ));
    }
    if result["events"]
        .as_array()
        .is_none_or(|events| !events.is_empty())
        || result["drops"]
            .as_array()
            .is_none_or(|drops| !drops.is_empty())
    {
        return Err(format!(
            "Cross-frame drag dispatched page input before rejection: {result}"
        ));
    }
    driver
        .release(view)
        .await
        .map_err(|error| error.to_string())?;
    windows::set_user_input_enabled(view, true).await?;
    Ok(json!({
        "unsupported_before_input": true,
        "events": result["events"],
        "drops": result["drops"],
    }))
}

#[cfg(windows)]
async fn verify(view: &tauri::Webview) -> Result<serde_json::Value, String> {
    use serde_json::json;
    loop {
        if evaluate(view, "document.readyState === 'complete' && !!window.smoke").await? == true {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    windows::set_user_input_enabled(view, false).await?;
    if context_menus_enabled(view).await? {
        return Err("Native context menus remained enabled during Agent input".into());
    }
    for selector in ["#field", "#submit"] {
        let point = evaluate(view, &format!("(() => {{ const r = document.querySelector({selector:?}).getBoundingClientRect(); return {{ x:r.x+r.width/2, y:r.y+r.height/2 }}; }})()")).await?;
        for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
            windows::protocol_call(
                view,
                "Input.dispatchMouseEvent",
                json!({
                    "type":kind,"x":point["x"],"y":point["y"],
                    "button":if kind == "mouseMoved" { "none" } else { "left" },
                    "clickCount":if kind == "mouseMoved" { 0 } else { 1 },
                }),
            )
            .await?;
        }
        if selector == "#field" {
            windows::protocol_call(view, "Input.insertText", json!({"text":"Nomi 中文输入"})).await?;
        }
    }
    windows::protocol_call(
        view,
        "Input.dispatchKeyEvent",
        json!({"type":"keyDown","key":"Tab","code":"Tab","windowsVirtualKeyCode":9}),
    )
    .await?;
    windows::protocol_call(
        view,
        "Input.dispatchKeyEvent",
        json!({"type":"keyUp","key":"Tab","code":"Tab","windowsVirtualKeyCode":9}),
    )
    .await?;
    let scroll_point = evaluate(view, "(() => { const r=document.querySelector('#scroller').getBoundingClientRect(); return {x:r.x+20,y:r.y+20}; })()").await?;
    windows::protocol_call(view, "Input.dispatchMouseEvent", json!({"type":"mouseWheel","x":scroll_point["x"],"y":scroll_point["y"],"deltaY":150,"deltaX":0})).await?;
    loop {
        if evaluate(view, "document.querySelector('#scroller').scrollTop > 0").await? == true {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let drag_point = evaluate(view, "(() => { const r=document.querySelector('#drag').getBoundingClientRect(); return {x:r.x+20,y:r.y+20}; })()").await?;
    let x = drag_point["x"].as_f64().ok_or("Missing drag point")?;
    let y = drag_point["y"].as_f64().ok_or("Missing drag point")?;
    windows::protocol_call(
        view,
        "Input.dispatchMouseEvent",
        json!({"type":"mouseMoved","x":x,"y":y}),
    )
    .await?;
    windows::protocol_call(
        view,
        "Input.dispatchMouseEvent",
        json!({"type":"mousePressed","x":x,"y":y,"button":"left","buttons":1,"clickCount":1}),
    )
    .await?;
    for delta in [10.0, 30.0, 70.0] {
        windows::protocol_call(
            view,
            "Input.dispatchMouseEvent",
            json!({"type":"mouseMoved","x":x+delta,"y":y,"button":"left","buttons":1}),
        )
        .await?;
    }
    windows::protocol_call(
        view,
        "Input.dispatchMouseEvent",
        json!({"type":"mouseReleased","x":x+70.0,"y":y,"button":"left","buttons":0,"clickCount":1}),
    )
    .await?;
    let value = evaluate(view, "({ value:document.querySelector('#field').value, clicks:smoke.clicks, events:smoke.events, trusted:smoke.events.every(e=>e.trusted), childLabel:'browser-smoke' })").await?;
    if value["value"] != "Nomi 中文输入" || value["clicks"] != 1 || value["trusted"] != true {
        return Err(format!("Native input assertions failed: {value}"));
    }
    let drag_verified = evaluate(view, "smoke.capturedMoves > 0").await?;
    if drag_verified != true {
        return Err("Native pointer capture/drag failed.".to_owned());
    }
    let denied = evaluate(
        view,
        "window.__TAURI_INTERNALS__.invoke('smoke_forbidden').then(()=>false,()=>true)",
    )
    .await?;
    if denied != true {
        return Err("External browser invoked an application command.".to_owned());
    }
    let identity = evaluate(view, "window.smoke.identity = crypto.randomUUID()").await?;
    view.hide().map_err(|error| error.to_string())?;
    view.set_size(tauri::LogicalSize::new(900.0, 600.0))
        .map_err(|error| error.to_string())?;
    view.show().map_err(|error| error.to_string())?;
    if evaluate(view, "window.smoke.identity").await? != identity {
        return Err("Hide/resize/show replaced the native document.".to_owned());
    }
    windows::set_user_input_enabled(view, true).await?;
    if !context_menus_enabled(view).await? {
        return Err("Native context menus were not restored for user input".into());
    }
    Ok(
        json!({"input":value,"custom_ipc_denied":denied,"native_lock_and_unlock":true,"nested_wheel":true,"pointer_capture_drag":drag_verified,"hide_resize_show_preserves_document":true}),
    )
}

#[cfg(windows)]
async fn context_menus_enabled(view: &tauri::Webview) -> Result<bool, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        // SAFETY: reads the child controller's settings on its owning UI thread.
        let result = unsafe {
            platform
                .controller()
                .CoreWebView2()
                .and_then(|core| core.Settings())
                .and_then(|settings| {
                    let mut enabled = ::windows::core::BOOL::default();
                    settings.AreDefaultContextMenusEnabled(&mut enabled)?;
                    Ok(enabled.as_bool())
                })
        }
        .map_err(|_| "Cannot read native context menu policy".to_owned());
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;
    rx.await.map_err(|e| e.to_string())?
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "This WebView2 smoke requires Windows. macOS requires its native conformance runner."
    );
    std::process::exit(1);
}

#[cfg(windows)]
async fn verify_workspace(app: &tauri::AppHandle, url: &str) -> Result<serde_json::Value, String> {
    use nomifun_browser_platform::{
        run_guard::{BrowserInputState, RunAdmissionError},
        runtime::{
            BrowserProfile, BrowserTabCommand, BrowserTabLifecycle,
            WorkspaceError,
        },
        workspace::BrowserResourceService,
    };
    use std::sync::Arc;
    let service =
        Arc::new(BrowserResourceService::new(Arc::new(host::DesktopBrowserHost::new(app.clone()))));
    let authority = browser_resource_fixture::authority(
        "smoke-user",
        "smoke-conversation",
        "native-webview2-v2",
    );
    let key = authority.key();
    let workspace = service
        .ensure(
            authority,
            BrowserProfile::Ephemeral,
        )
        .await
        .map_err(|e| e.to_string())?;
    if workspace
        .snapshot()
        .await
        .map_err(|e| e.to_string())?
        .runtime
        .is_some()
    {
        return Err("Workspace launched a browser before first use.".into());
    }
    let run = workspace.begin_run().await.map_err(|e| e.to_string())?;
    let created = workspace
        .agent_command(&run, BrowserTabCommand::Create { url: url.into() })
        .await
        .map_err(|e| e.to_string())?;
    workspace
        .set_surface(
            nomifun_browser_platform::runtime::BrowserSurfaceBounds {
                x: 20.0,
                y: 60.0,
                width: 1060.0,
                height: 640.0,
            },
            true,
            Default::default(),
        )
        .await
        .map_err(|e| e.to_string())?;
    let tab_id = created
        .active_tab_id
        .clone()
        .ok_or("No active native tab")?;
    loop {
        let snapshot = workspace.snapshot().await.map_err(|e| e.to_string())?;
        let runtime = snapshot.runtime.ok_or("Missing native runtime")?;
        if runtime.tabs.iter().any(|tab| {
            tab.target.tab_id == tab_id
                && tab.lifecycle == BrowserTabLifecycle::Ready
                && tab.url == url
        }) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let observation = workspace
        .observe(&run, Some(tab_id.clone()))
        .await
        .map_err(|error| error.to_string())?;
    let disabled = observation
        .elements
        .iter()
        .find(|element| element.name == "不可点击")
        .ok_or("Snapshot omitted disabled control")?
        .reference
        .clone();
    if !matches!(
        workspace
            .act(
                &run,
                nomifun_browser_platform::runtime::BrowserAction::click(disabled)
            )
            .await,
        Err(WorkspaceError::NotActionable)
    ) {
        return Err("Disabled control bypassed actionability checks.".into());
    }
    let textbox = observation
        .elements
        .iter()
        .find(|element| element.role == "textbox")
        .ok_or_else(|| format!("Semantic snapshot omitted textbox: {}", observation.content))?
        .reference
        .clone();
    workspace
        .act(
            &run,
            nomifun_browser_platform::runtime::BrowserAction::Type {
                element: textbox.clone(),
                text: "语义输入验证".into(),
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    if !matches!(
        workspace
            .act(
                &run,
                nomifun_browser_platform::runtime::BrowserAction::click(textbox)
            )
            .await,
        Err(WorkspaceError::StaleObservation)
    ) {
        return Err("Consumed observation was incorrectly reused.".into());
    }
    let observation = workspace
        .observe(&run, Some(tab_id.clone()))
        .await
        .map_err(|error| error.to_string())?;
    if observation.content.contains("语义输入验证") {
        return Err("Observation exposed an editable field value.".into());
    }
    let button = observation
        .elements
        .iter()
        .find(|element| element.role == "button")
        .ok_or("Semantic snapshot omitted button")?
        .reference
        .clone();
    workspace
        .act(
            &run,
            nomifun_browser_platform::runtime::BrowserAction::click(button),
        )
        .await
        .map_err(|error| error.to_string())?;
    let native_view = {
        use tauri::Manager;
        app.get_webview(&tab_id)
            .ok_or("Semantic action lost its native view")?
    };
    let semantic_result = evaluate(&native_view,"({value:document.querySelector('#field').value,clicks:smoke.clicks,trusted:smoke.events.every(e=>e.trusted)})").await?;
    if context_menus_enabled(&native_view).await? {
        return Err("Workspace run allowed native context menus".into());
    }
    if semantic_result["value"] != "语义输入验证"
        || semantic_result["clicks"] != 1
        || semantic_result["trusted"] != true
    {
        return Err(format!("Semantic native input failed: {semantic_result}"));
    }
    let key_observation = workspace
        .observe(&run, Some(tab_id.clone()))
        .await
        .map_err(|error| error.to_string())?;
    let focused_button = key_observation
        .elements
        .iter()
        .find(|element| element.role == "button" && element.focused)
        .ok_or("Button did not retain native focus")?
        .reference
        .clone();
    workspace
        .act(
            &run,
            nomifun_browser_platform::runtime::BrowserAction::Press {
                element: focused_button,
                keys: "Enter".into(),
            },
        )
        .await
        .map_err(|error| error.to_string())?;
    if evaluate(&native_view, "smoke.clicks").await? != 2 {
        return Err("Enter dispatched an extra click before key input.".into());
    }
    use nomifun_browser_platform::runtime::{BrowserAction, BrowserMouseButton};
    let observation = workspace
        .observe(&run, Some(tab_id.clone()))
        .await
        .map_err(|e| e.to_string())?;
    let gesture_ref = observation
        .elements
        .iter()
        .find(|el| el.name == "鼠标手势验证")
        .ok_or("Gesture target absent from semantic observation")?
        .reference
        .clone();
    for click_count in [0, 3, 255] {
        if !matches!(
            workspace
                .act(
                    &run,
                    BrowserAction::Click {
                        element: gesture_ref.clone(),
                        button: BrowserMouseButton::Left,
                        click_count,
                    }
                )
                .await,
            Err(WorkspaceError::UnsupportedAction)
        ) {
            return Err("Unsupported click count entered the native input pipeline".into());
        }
    }
    for (button, click_count) in [
        (BrowserMouseButton::Left, 2),
        (BrowserMouseButton::Right, 1),
        (BrowserMouseButton::Middle, 1),
    ] {
        let observation = workspace
            .observe(&run, Some(tab_id.clone()))
            .await
            .map_err(|e| e.to_string())?;
        let element = observation
            .elements
            .iter()
            .find(|el| el.name == "鼠标手势验证")
            .ok_or("Gesture target absent from semantic observation")?
            .reference
            .clone();
        workspace
            .act(
                &run,
                BrowserAction::Click {
                    element,
                    button,
                    click_count,
                },
            )
            .await
            .map_err(|e| e.to_string())?;
    }
    let gestures = evaluate(
        &native_view,
        "smoke.events.filter(e=>e.target==='gestures')",
    )
    .await?;
    let events = gestures
        .as_array()
        .ok_or("Missing native gesture evidence")?;
    for (event_type, button, detail) in [
        ("dblclick", 0, Some(2)),
        ("contextmenu", 2, None),
        ("auxclick", 1, Some(1)),
    ] {
        if !events.iter().any(|event| {
            event["type"] == event_type
                && event["button"] == button
                && detail.is_none_or(|detail| event["detail"] == detail)
                && event["trusted"] == true
        }) {
            return Err(format!("Native {event_type} evidence missing: {gestures}"));
        }
    }
    for (button, mask, expected_downs) in [(0, 1, 2), (2, 2, 1), (1, 4, 1)] {
        let downs = events
            .iter()
            .filter(|e| e["type"] == "mousedown" && e["button"] == button && e["buttons"] == mask)
            .count();
        let ups = events
            .iter()
            .filter(|e| e["type"] == "mouseup" && e["button"] == button && e["buttons"] == 0)
            .count();
        if downs != expected_downs || ups != expected_downs {
            return Err(format!("Native mouse press/release imbalance: {gestures}"));
        }
    }
    if events.iter().any(|event| event["trusted"] != true) {
        return Err("Synthetic event entered native gesture evidence".into());
    }
    let observation = workspace
        .observe(&run, Some(tab_id.clone()))
        .await
        .map_err(|e| e.to_string())?;
    let replacing = observation
        .elements
        .iter()
        .find(|el| el.name == "首次点击后替换")
        .ok_or("Missing replacing gesture target")?
        .reference
        .clone();
    if !matches!(
        workspace
            .act(
                &run,
                BrowserAction::Click {
                    element: replacing,
                    button: BrowserMouseButton::Left,
                    click_count: 2,
                }
            )
            .await,
        Err(WorkspaceError::ActionInterrupted)
    ) {
        return Err("Double-click did not stop when its first click replaced the target".into());
    }
    if evaluate(&native_view, "smoke.replacementClicks").await? != 1 {
        return Err("The second click was sent to a replacement control".into());
    }
    let selection_evidence = verify_selections(&workspace, &run, &tab_id, &native_view).await?;
    let previous_run_observation = workspace
        .observe(&run, Some(tab_id.clone()))
        .await
        .map_err(|error| error.to_string())?;
    let previous_run_reference = previous_run_observation
        .elements
        .iter()
        .find(|element| element.role == "button")
        .ok_or("Missing button reference")?
        .reference
        .clone();
    let rejected = workspace
        .user_command(BrowserTabCommand::Create { url: url.into() })
        .await;
    if !matches!(
        rejected,
        Err(WorkspaceError::Admission(
            RunAdmissionError::UserInputLocked
        ))
    ) {
        return Err("User tab command entered an active Agent run.".into());
    }
    workspace
        .finish_run(&run)
        .await
        .map_err(|e| e.to_string())?;
    if !context_menus_enabled(&native_view).await? {
        return Err("Workspace terminal did not restore native context menus".into());
    }
    let snapshot = workspace.snapshot().await.map_err(|e| e.to_string())?;
    if snapshot.run.input_state != BrowserInputState::UserReady
        || snapshot
            .runtime
            .as_ref()
            .and_then(|runtime| runtime.active_tab_id.as_ref())
            != Some(&tab_id)
    {
        return Err("Run finish lost the native browser page.".into());
    }
    let next = workspace.begin_run().await.map_err(|e| e.to_string())?;
    if !matches!(
        workspace
            .act(
                &next,
                nomifun_browser_platform::runtime::BrowserAction::click(previous_run_reference)
            )
            .await,
        Err(WorkspaceError::StaleObservation)
    ) {
        return Err("A previous Agent turn's element reference was reused.".into());
    }
    let same = workspace.snapshot().await.map_err(|e| e.to_string())?;
    if same
        .runtime
        .as_ref()
        .and_then(|runtime| runtime.active_tab_id.as_ref())
        != Some(&tab_id)
    {
        return Err("Next Agent turn received a different browser.".into());
    }
    workspace
        .finish_run(&next)
        .await
        .map_err(|e| e.to_string())?;
    let old_target = workspace
        .snapshot()
        .await
        .map_err(|e| e.to_string())?
        .runtime
        .ok_or("Missing native runtime")?
        .tabs[0]
        .target
        .clone();
    let next_url = format!("{url}next");
    workspace
        .user_command(BrowserTabCommand::Navigate {
            target: old_target.clone(),
            url: next_url.clone(),
        })
        .await
        .map_err(|e| e.to_string())?;
    let navigated = wait_workspace_page(&workspace, &next_url).await?;
    if navigated.document_generation <= old_target.document_generation {
        return Err("Navigation did not invalidate the document generation.".into());
    }
    if !matches!(
        workspace
            .user_command(BrowserTabCommand::Reload { target: old_target })
            .await,
        Err(WorkspaceError::StaleTarget)
    ) {
        return Err("An old page target was allowed to control a new document.".into());
    }
    workspace
        .user_command(BrowserTabCommand::Back { target: navigated })
        .await
        .map_err(|e| e.to_string())?;
    let previous = wait_workspace_page(&workspace, url).await?;
    workspace
        .user_command(BrowserTabCommand::Forward { target: previous })
        .await
        .map_err(|e| e.to_string())?;
    wait_workspace_page(&workspace, &next_url).await?;
    // WebView2 can briefly retain profile files after controller close.
    loop {
        match service.close_idle(key.clone(),workspace.runtime_generation()).await {
            Ok(()) => break,
            Err(WorkspaceError::NativeCommandFailed) => {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await
            }
            Err(error) => return Err(error.to_string()),
        }
    }
    if !matches!(workspace.begin_run().await,Err(WorkspaceError::WorkspaceClosed)) {
        return Err("Retired native workspace accepted another Agent run".into());
    }
    let replacement=service.ensure(browser_resource_fixture::authority("smoke-user","smoke-conversation","replacement-native-provider"),BrowserProfile::Ephemeral).await.map_err(|error|error.to_string())?;
    if replacement.runtime_generation()<=workspace.runtime_generation() {return Err("Browser rebuild reused the native runtime generation".into());}
    replacement.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(|error|format!("Replacement native tab creation: {error}"))?;
    let rebuilt=wait_workspace_page(&replacement,url).await?;
    if rebuilt.tab_id==tab_id {return Err("Browser rebuild reused a destroyed native tab".into());}
    let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
    loop {
        match service.shutdown().await {
            Ok(())=>break,
            Err(error) if tokio::time::Instant::now()>=deadline=>return Err(format!("Replacement native cleanup: {error}")),
            Err(_)=>tokio::time::sleep(std::time::Duration::from_millis(50)).await,
        }
    }
    Ok(
        serde_json::json!({"lazy_runtime":true,"user_commands_locked_during_run":true,"same_tab_across_runs":true,"native_close_and_profile_cleanup":true,"idle_close_and_new_provider_generation":true,"stale_document_rejected":true,"history_back_forward":true,"semantic_observe_type_click":semantic_result,"consumed_observation_rejected":true,"editable_values_redacted":true,"native_key_press_once":true,"disabled_control_rejected":true,"previous_run_ref_rejected":true,"native_mouse_gestures":gestures,"double_click_replacement_stopped":true,"invalid_click_counts_rejected":true,"selection":selection_evidence}),
    )
}

#[cfg(windows)]
async fn verify_runtime_locks(
    app: &tauri::AppHandle,
    url: &str,
) -> Result<serde_json::Value, String> {
    use nomifun_browser_platform::runtime::*;
    use serde_json::json;
    use tauri::Manager;
    let runtime = host::DesktopBrowserHost::for_transport_conformance(app.clone())
        .create(CreateBrowserRuntime {
            key: browser_resource_fixture::key("native-lock-smoke", "native-lock-smoke"),
            runtime_generation: 1,
            profile: BrowserProfile::Ephemeral,
            user_input_enabled: false,
        })
        .await
        .map_err(|error| error.to_string())?;
    let mut created_child = None;
    let result: Result<serde_json::Value,String> = async {
    let cancel = tokio_util::sync::CancellationToken::new();
    let created = runtime
        .execute(
            BrowserTabCommand::Create { url: url.into() },
            cancel.clone(),
        )
        .await
        .map_err(|error| error.to_string())?;
    let source = created
        .active_tab_id
        .ok_or("Missing lock-smoke source tab")?;
    let view = app
        .get_webview(&source)
        .ok_or("Missing lock-smoke native view")?;
    let bounds = BrowserSurfaceBounds {
        x: 20.0,
        y: 60.0,
        width: 1000.0,
        height: 600.0,
    };
    runtime
        .surface()
        .ok_or("Missing native surface")?
        .set_surface(bounds, true, Default::default())
        .await
        .map_err(|error| error.to_string())?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if evaluate(
            &view,
            "!!window.popupNonce && document.readyState==='complete'",
        )
        .await?
            == true
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("Runtime lock fixture did not load".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let before = evaluate(&view, "({width:innerWidth,height:innerHeight})").await?;
    let native_before = native_bounds(&view).await?;
    let mut subscription = windows::popup::PopupSubscription::listen(&view).await?;
    let automation = runtime.automation().ok_or("Missing native automation")?;
    let observed = automation
        .observe(Some(source.clone()), cancel.clone())
        .await
        .map_err(|error| error.to_string())?;
    let element = observed
        .elements
        .iter()
        .find(|element| element.name == "Open real popup")
        .ok_or("Missing native popup trigger")?
        .reference
        .clone();
    let consumer = async {
        let request = tokio::time::timeout(std::time::Duration::from_secs(3), subscription.next())
            .await
            .map_err(|_| "Runtime popup event did not arrive")?
            .ok_or("Popup event stream closed")?;
        let snapshot =
            tokio::time::timeout(std::time::Duration::from_millis(500), runtime.snapshot())
                .await
                .map_err(|_| "Registry snapshot blocked behind native input")?
                .map_err(|error| error.to_string())?;
        if snapshot.tabs.len() != 1 {
            return Err("Unexpected registry during input".into());
        }
        let hidden = BrowserSurfaceBounds {
            width: 880.0,
            height: 560.0,
            ..bounds
        };
        tokio::time::timeout(
            std::time::Duration::from_millis(500),
            runtime.surface().unwrap().set_surface(hidden, false, Default::default()),
        )
        .await
        .map_err(|_| "Hiding a surface blocked behind native input")?
        .map_err(|error| error.to_string())?;
        if native_bounds(&view).await? != native_before {
            return Err("Hiding resized the native document during its input operation".into());
        }
        let resize = runtime.surface().unwrap().set_surface(hidden, true, Default::default());
        tokio::pin!(resize);
        if tokio::time::timeout(std::time::Duration::from_millis(100), &mut resize)
            .await
            .is_ok()
        {
            return Err("Visible resize crossed an unsettled page input".into());
        }
        let second = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            runtime.execute(
                BrowserTabCommand::Create { url: url.into() },
                cancel.clone(),
            ),
        )
        .await
        .map_err(|_| "A new tab was blocked behind the opener input")?
        .map_err(|error| error.to_string())?;
        if second.tabs.len() != 2 {
            return Err("New tab was not retained by its runtime".into());
        }
        // Low-level binding is still explicit here: production popup admission
        // is a separate integration step, not silently enabled by this test.
        let child = request.create_child(|builder| builder).await?;
        created_child=Some(child.clone());
        windows::set_user_input_enabled(&child, false).await?;
        request.complete(&child).await?;
        tokio::time::timeout(std::time::Duration::from_secs(3), &mut resize)
            .await
            .map_err(|_| "Resize did not resume after input settlement")?
            .map_err(|error| error.to_string())?;
        Ok::<_, String>(child)
    };
    let (action, child) = tokio::join!(
        automation.act(BrowserAction::click(element), cancel.clone()),
        consumer
    );
    action.map_err(|error| format!("Runtime popup click failed: {error}"))?;
    let child = child?;
    if evaluate(&view, "({width:innerWidth,height:innerHeight})").await? != before {
        return Err("The original page viewport changed while a popup input was pending".into());
    }
    let active=runtime.snapshot().await.map_err(|error|error.to_string())?.active_tab_id.ok_or("Missing resized active tab")?;
    let active_view=app.get_webview(&active).ok_or("Missing resized native view")?;
    if evaluate(&active_view,"({width:innerWidth,height:innerHeight})").await?!=json!({"width":880,"height":560}) {
        return Err("Deferred resize did not target the newly active page".into());
    }
    windows::close_native_view(&child).await?;
    created_child=None;
    let observed=automation.observe(Some(source.clone()),cancel.clone()).await.map_err(|error|error.to_string())?;
    let element=observed.elements.iter().find(|element|element.name=="Open real popup")
        .ok_or("Missing second popup trigger")?.reference.clone();
    let consumer=async {
        let request=tokio::time::timeout(std::time::Duration::from_secs(3),subscription.next()).await
            .map_err(|_|"Second runtime popup did not arrive")?.ok_or("Popup stream closed")?;
        let layout_cancel=tokio_util::sync::CancellationToken::new();
        let resize=runtime.surface().unwrap().set_surface(bounds,true,layout_cancel.clone());
        tokio::pin!(resize);
        if tokio::time::timeout(std::time::Duration::from_millis(100),&mut resize).await.is_ok() {
            return Err("Second resize crossed pending input".into());
        }
        layout_cancel.cancel();
        tokio::time::timeout(std::time::Duration::from_millis(500),&mut resize).await
            .map_err(|_|"Layout cancellation waited for the browser input")?.map_err(|error|error.to_string())?;
        if cancel.is_cancelled() || subscription.pending_count().await?!=1 {
            return Err("Layout cancellation changed browser input authority".into());
        }
        tokio::time::timeout(std::time::Duration::from_millis(500),runtime.surface().unwrap().set_surface(bounds,false, Default::default())).await
            .map_err(|_|"A queued resize prevented a newer hide")?.map_err(|error|error.to_string())?;
        let child=request.create_child(|builder|builder).await?;
        created_child=Some(child.clone());
        windows::set_user_input_enabled(&child,false).await?;
        request.complete(&child).await?;
        Ok::<_,String>(())
    };
    let (action,result)=tokio::join!(automation.act(BrowserAction::click(element),cancel.clone()),consumer);
    action.map_err(|error|error.to_string())?;
    result?;
    if native_visible(&view).await? {return Err("An old resize resurrected a hidden browser surface".into());}
    let (entered_tx,entered_rx)=tokio::sync::oneshot::channel();
    let (resume_tx,resume_rx)=std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        let _=entered_tx.send(());
        let _=resume_rx.recv_timeout(std::time::Duration::from_secs(2));
    }).map_err(|error|error.to_string())?;
    entered_rx.await.map_err(|error|error.to_string())?;
    let queued_cancel=tokio_util::sync::CancellationToken::new();
    let queued=runtime.surface().unwrap().set_surface(bounds,true,queued_cancel.clone());
    tokio::pin!(queued);
    if tokio::time::timeout(std::time::Duration::from_millis(50),&mut queued).await.is_ok() {
        return Err("Native layout acknowledged before its UI task ran".into());
    }
    queued_cancel.cancel();
    let _=resume_tx.send(());
    queued.await.map_err(|error|error.to_string())?;
    if native_visible(&view).await? {return Err("A cancelled queued UI task showed the browser".into());}
    runtime.surface().unwrap().set_surface(bounds,true,Default::default()).await.map_err(|error|error.to_string())?;
    let target=runtime.snapshot().await.map_err(|error|error.to_string())?.tabs.into_iter()
        .find(|tab|tab.target.tab_id==source).ok_or("Missing delayed navigation source")?.target;
    let mut slow=url::Url::parse(url).map_err(|error|error.to_string())?;
    slow.set_path("/slow-navigation");
    DELAYED_NAVIGATION_STARTED.store(false,std::sync::atomic::Ordering::SeqCst);
    let finished=std::sync::atomic::AtomicBool::new(false);
    let navigate=async {
        let result=runtime.execute(BrowserTabCommand::Navigate {target,url:slow.to_string()},cancel.clone()).await;
        finished.store(true,std::sync::atomic::Ordering::SeqCst);
        result
    };
    let during_navigation=async {
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(1);
        loop {
            if DELAYED_NAVIGATION_STARTED.load(std::sync::atomic::Ordering::SeqCst) {break}
            if tokio::time::Instant::now()>=deadline {return Err("Delayed navigation did not start".to_owned());}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        tokio::time::timeout(std::time::Duration::from_millis(250),runtime.snapshot()).await
            .map_err(|_|"Navigation held the registry while waiting for its response")?.map_err(|error|error.to_string())?;
        tokio::time::timeout(std::time::Duration::from_millis(250),runtime.surface().unwrap().set_surface(bounds,false,Default::default())).await
            .map_err(|_|"Navigation prevented a native hide")?.map_err(|error|error.to_string())?;
        if finished.load(std::sync::atomic::Ordering::SeqCst) || native_visible(&view).await? {
            return Err("Hide did not complete while navigation was pending".to_owned());
        }
        Ok(())
    };
    let (navigation,hidden)=tokio::join!(navigate,during_navigation);
    navigation.map_err(|error|error.to_string())?;
    hidden?;
    let target=runtime.snapshot().await.map_err(|error|error.to_string())?.tabs.into_iter()
        .find(|tab|tab.target.tab_id==source).ok_or("Missing cancellation source")?.target;
    slow.set_query(Some("cancel-navigation"));
    DELAYED_NAVIGATION_STARTED.store(false,std::sync::atomic::Ordering::SeqCst);
    let cancelled=tokio_util::sync::CancellationToken::new();
    cancelled.cancel();
    if !matches!(runtime.execute(BrowserTabCommand::Navigate {target:target.clone(),url:slow.to_string()},cancelled).await,
        Err(WorkspaceError::Admission(nomifun_browser_platform::run_guard::RunAdmissionError::Cancelled)))
        || DELAYED_NAVIGATION_STARTED.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("Already-cancelled navigation was submitted".into());
    }
    let cancelled=tokio_util::sync::CancellationToken::new();
    let stop=async {
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(1);
        while !DELAYED_NAVIGATION_STARTED.load(std::sync::atomic::Ordering::SeqCst) {
            if tokio::time::Instant::now()>=deadline {return Err("Cancellable navigation did not reach the server".to_owned());}
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let stopped_at=tokio::time::Instant::now();
        cancelled.cancel();
        Ok(stopped_at)
    };
    let (navigation,stopped_at)=tokio::join!(runtime.execute(BrowserTabCommand::Navigate {target,url:slow.to_string()},cancelled.clone()),stop);
    if !matches!(navigation,Err(WorkspaceError::Admission(nomifun_browser_platform::run_guard::RunAdmissionError::Cancelled)))
        || stopped_at?.elapsed()>std::time::Duration::from_secs(1) {
        return Err(format!("Navigation did not stop and settle before its delayed response: {navigation:?}"));
    }
    let target=runtime.snapshot().await.map_err(|error|error.to_string())?.tabs.into_iter()
        .find(|tab|tab.target.tab_id==source).ok_or("Missing recovery source")?.target;
    runtime.execute(BrowserTabCommand::Navigate {target,url:url.into()},cancel.clone()).await.map_err(|error|error.to_string())?;
    let before_create=runtime.snapshot().await.map_err(|error|error.to_string())?;
    runtime.surface().unwrap().set_surface(bounds,true,Default::default()).await.map_err(|error|error.to_string())?;
    let before_labels:std::collections::BTreeSet<_>=app.get_window("main").ok_or("Missing fixture window")?.webviews()
        .into_iter().map(|view|view.label().to_owned()).collect();
    slow.set_query(Some("cancel-create"));
    DELAYED_NAVIGATION_STARTED.store(false,std::sync::atomic::Ordering::SeqCst);
    let cancelled=tokio_util::sync::CancellationToken::new();
    let stop=async {
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        while !DELAYED_NAVIGATION_STARTED.load(std::sync::atomic::Ordering::SeqCst) {
            if tokio::time::Instant::now()>=deadline {return Err("New tab did not reach its delayed navigation".to_owned());}
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let during=tokio::time::timeout(std::time::Duration::from_millis(250),runtime.snapshot()).await
            .map_err(|_|"Creation held the tab registry")?.map_err(|error|error.to_string())?;
        if during.tabs.len()!=before_create.tabs.len()+1 {return Err("Creating tab was not retained before navigation".to_owned());}
        tokio::time::timeout(std::time::Duration::from_millis(250),runtime.surface().unwrap().set_surface(bounds,false,Default::default())).await
            .map_err(|_|"Creation blocked hiding the native surface")?.map_err(|error|error.to_string())?;
        if native_visible(&view).await? {return Err("Existing surface remained visible during creation".to_owned());}
        let stopped_at=tokio::time::Instant::now();
        cancelled.cancel();
        Ok(stopped_at)
    };
    let (created,stopped_at)=tokio::join!(runtime.execute(BrowserTabCommand::Create {url:slow.to_string()},cancelled.clone()),stop);
    if !matches!(created,Err(WorkspaceError::Admission(nomifun_browser_platform::run_guard::RunAdmissionError::Cancelled)))
        || stopped_at?.elapsed()>std::time::Duration::from_secs(1) {
        return Err(format!("New-tab cancellation did not settle: {created:?}"));
    }
    let after_create=runtime.snapshot().await.map_err(|error|error.to_string())?;
    let after_labels:std::collections::BTreeSet<_>=app.get_window("main").ok_or("Missing fixture window")?.webviews()
        .into_iter().map(|view|view.label().to_owned()).collect();
    if after_create.active_tab_id!=before_create.active_tab_id || after_create.tabs.len()!=before_create.tabs.len() || after_labels!=before_labels {
        return Err("Cancelled creation left a native view or changed the active tab".into());
    }
    let recovered=runtime.execute(BrowserTabCommand::Create {url:url.into()},cancel.clone()).await.map_err(|error|error.to_string())?;
    if recovered.tabs.len()!=before_create.tabs.len()+1 {return Err("New tab could not be created after cancellation".into());}
    runtime
        .release_pressed_input()
        .await
        .map_err(|error| error.to_string())?;
    runtime
        .unlock_user_input()
        .await
        .map_err(|error| error.to_string())?;
    if let Some(child)=created_child.take() {windows::close_native_view(&child).await?;}
    slow.set_query(Some("close-during-create"));
    DELAYED_NAVIGATION_STARTED.store(false,std::sync::atomic::Ordering::SeqCst);
    let close=async {
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        while !DELAYED_NAVIGATION_STARTED.load(std::sync::atomic::Ordering::SeqCst) {
            if tokio::time::Instant::now()>=deadline {return Err("Closing creation did not reach the server".to_owned());}
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let result=runtime.close().await;
        // A transient profile file lock remains owned and is retried below.
        if let Err(error)=result {
            if error!=WorkspaceError::NativeCommandFailed {return Err(error.to_string());}
        }
        Ok(())
    };
    let (created,closed)=tokio::join!(runtime.execute(BrowserTabCommand::Create {url:slow.to_string()},cancel.clone()),close);
    closed?;
    if !matches!(created,Err(WorkspaceError::WorkspaceClosed)) {return Err(format!("Creation survived runtime close: {created:?}"));}
    Ok(
        json!({"snapshot_during_input":true,"hide_during_input":true,"hide_preserves_viewport":true,
        "resize_waits_for_input":true,"new_tab_during_input":true,"active_tab_change_rechecked":true,
        "input_settles_before_unlock":true,"native_runtime_cleanup":true,"newer_hide_supersedes_queued_resize":true,"layout_cancel_does_not_cancel_input":true,"queued_ui_cancel_checked":true,"navigation_does_not_block_hide":true,"navigation_cancel_stops_native_load":true,"navigation_recovers_after_cancel":true,"cancelled_creation_closes_native_candidate":true,"creation_recovers_after_cancel":true,"creation_does_not_block_hide":true,"close_settles_creation_before_cleanup":true}),
    )
    }.await;
    if let Some(child) = created_child {
        if app.get_webview(child.label()).is_some() {
            windows::close_native_view(&child).await?;
        }
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match runtime.close().await {
            Ok(()) => break,
            Err(error) if tokio::time::Instant::now() >= deadline => {
                return Err(format!("Runtime lock fixture cleanup: {error}"));
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
        }
    }
    result
}

#[cfg(windows)]
async fn verify_managed_popup(
    app: &tauri::AppHandle,
    url: &str,
) -> Result<serde_json::Value, String> {
    use nomifun_browser_platform::{runtime::*, workspace::BrowserResourceService};
    use std::sync::Arc;
    use tauri::Manager;
    let service =
        BrowserResourceService::new(Arc::new(host::DesktopBrowserHost::new(app.clone())));
    let authority = browser_resource_fixture::authority(
        "managed-popup",
        "managed-popup",
        "native-popup-provider",
    );
    let key = authority.key();
    let workspace = service
        .ensure(
            authority,
            BrowserProfile::Ephemeral,
        )
        .await
        .map_err(|error| error.to_string())?;
    let result=async {
        let run=workspace.begin_run().await.map_err(|error|error.to_string())?;
        let created=workspace.agent_command(&run,BrowserTabCommand::Create {url:url.into()}).await.map_err(|error|error.to_string())?;
        let source=created.active_tab_id.ok_or("Missing popup opener")?;
        let view=app.get_webview(&source).ok_or("Missing popup native opener")?;
        workspace.set_surface(BrowserSurfaceBounds {x:20.0,y:60.0,width:1000.0,height:600.0},true,Default::default()).await.map_err(|error|error.to_string())?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
        loop {
            if evaluate(&view,"!!window.popupNonce && document.readyState==='complete'").await?==true {break}
            if tokio::time::Instant::now()>=deadline {return Err("Managed opener did not load".into())}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let nonce=evaluate(&view,"popupNonce").await?;
        let initial_tab=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing initial history")?.tabs
            .into_iter().find(|tab|tab.target.tab_id==source).ok_or("Missing initial source history")?;
        let observed=workspace.observe(&run,Some(source.clone())).await.map_err(|error|error.to_string())?;
        let spa=observed.elements.iter().find(|element|element.name=="Push SPA route").ok_or("Missing SPA control")?.reference.clone();
        workspace.act(&run,BrowserAction::click(spa)).await.map_err(|error|error.to_string())?;
        let mut spa_url=url::Url::parse(url).map_err(|error|error.to_string())?;
        spa_url.set_path("/spa-route");spa_url.set_query(Some("step=1"));spa_url.set_fragment(Some("native"));
        let changed=wait_navigation_metadata(&workspace,&source,spa_url.as_str(),true,false).await?;
        if changed.document_generation!=initial_tab.target.document_generation || evaluate(&view,"popupNonce").await?!=nonce {
            return Err("SPA history update replaced the document generation or instance".into());
        }
        workspace.agent_command(&run,BrowserTabCommand::Back {target:changed}).await.map_err(|error|error.to_string())?;
        let restored=wait_navigation_metadata(&workspace,&source,url,initial_tab.can_go_back,true).await?;
        workspace.agent_command(&run,BrowserTabCommand::Forward {target:restored}).await.map_err(|error|error.to_string())?;
        let changed=wait_navigation_metadata(&workspace,&source,spa_url.as_str(),true,false).await?;
        workspace.agent_command(&run,BrowserTabCommand::Back {target:changed}).await.map_err(|error|error.to_string())?;
        wait_navigation_metadata(&workspace,&source,url,initial_tab.can_go_back,true).await?;
        let observed=workspace.observe(&run,Some(source.clone())).await.map_err(|error|error.to_string())?;
        let geo=observed.elements.iter().find(|element|element.name=="Request geolocation").ok_or("Missing permission fixture control")?.reference.clone();
        workspace.act(&run,BrowserAction::click(geo)).await.map_err(|error|error.to_string())?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {
            let snapshot=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing permission metadata")?;
            let denied=snapshot.tabs.iter().find(|tab|tab.target.tab_id==source).is_some_and(|tab|tab.blocked_permissions.iter().any(|kind|kind=="geolocation"));
            if denied && evaluate(&view,"window.geoResult").await?=="denied-1" {break}
            if tokio::time::Instant::now()>=deadline {return Err(format!("Native permission was not denied and recorded: {snapshot:?}"))}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let observed=workspace.observe(&run,Some(source.clone())).await.map_err(|error|error.to_string())?;
        let diagnostic=observed.elements.iter().find(|element|element.name=="Produce browser diagnostics").ok_or("Missing diagnostics fixture control")?.reference.clone();
        workspace.act(&run,BrowserAction::click(diagnostic)).await.map_err(|error|error.to_string())?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {
            let snapshot=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing diagnostic snapshot")?;
            let tab=snapshot.tabs.iter().find(|tab|tab.target.tab_id==source).ok_or("Missing diagnostic tab")?;
            let entries=&tab.diagnostics.entries;
            if entries.iter().any(|entry|entry.message.contains("nomi-native-console")) && entries.iter().any(|entry|entry.kind=="page_error" && entry.message.contains("nomi-native-page-error")) && entries.iter().any(|entry|entry.kind=="network") {
                if entries.iter().any(|entry|entry.source_url.contains("not-in-metadata")) || evaluate(&view,"diagnosticGetterCalls").await?!=0 { return Err("Diagnostics leaked URL query or invoked an object getter".into()) }
                break;
            }
            if tokio::time::Instant::now()>=deadline {return Err(format!("Native diagnostics did not arrive: {entries:?}"))}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        native_fixture_click(&view,"#unique").await?;
        if workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing idle Agent popup state")?.tabs.len()!=1 {
            return Err("Popup without an active Agent operation bypassed the run scope".into());
        }
        let observed=workspace.observe(&run,Some(source.clone())).await.map_err(|error|error.to_string())?;
        let open=observed.elements.iter().find(|element|element.name=="Open real popup").ok_or("Missing popup control")?.reference.clone();
        workspace.act(&run,BrowserAction::click(open)).await.map_err(|error|format!("Managed popup action: {error}"))?;
        let popup=loop {
            let snapshot=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing popup runtime")?;
            if let Some(tab)=snapshot.tabs.iter().find(|tab|tab.target.tab_id!=source && tab.url.ends_with("/popup-child") && tab.lifecycle==BrowserTabLifecycle::Ready) {
                if snapshot.active_tab_id.as_ref()!=Some(&tab.target.tab_id) {return Err("Popup did not become the active conversation tab".into())}
                break tab.clone();
            }
            if tokio::time::Instant::now()>=deadline {return Err(format!("Managed popup was not admitted: {snapshot:?}"))}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        let popup_view=app.get_webview(&popup.target.tab_id).ok_or("Missing managed native popup")?;
        let proof=evaluate(&popup_view,"popupProof").await?;
        if proof["nonce"]!=nonce || proof["hasOpener"]!=true {return Err(format!("Managed popup lost its opener: {proof}"))}
        let popup_nonce=evaluate(&popup_view,"popupDocumentNonce").await?;
        let observed=workspace.observe(&run,Some(source.clone())).await.map_err(|error|error.to_string())?;
        let open=observed.elements.iter().find(|element|element.name=="Open real popup").ok_or("Missing repeat popup control")?.reference.clone();
        workspace.act(&run,BrowserAction::click(open)).await.map_err(|error|format!("Named popup reuse: {error}"))?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {
            let snapshot=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing reused popup runtime")?;
            if snapshot.tabs.len()!=2 || evaluate(&view,"popupReused").await?!=true {
                return Err(format!("Repeated window name created a different native WindowProxy or tab: {snapshot:?}"));
            }
            if snapshot.tabs.iter().any(|tab|tab.target.tab_id==popup.target.tab_id && tab.lifecycle==BrowserTabLifecycle::Ready
                && tab.target.document_generation>popup.target.document_generation)
                && evaluate(&popup_view,"window.popupDocumentNonce").await?!=popup_nonce { break; }
            if tokio::time::Instant::now()>=deadline {return Err(format!("Named popup did not navigate the existing native document: {snapshot:?}"))}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        if context_menus_enabled(&popup_view).await? {return Err("Popup did not inherit the Agent input lock".into())}
        let observed=workspace.observe(&run,Some(popup.target.tab_id.clone())).await.map_err(|error|error.to_string())?;
        let reply=observed.elements.iter().find(|element|element.name=="Reply from popup").ok_or("Missing managed popup reply")?.reference.clone();
        workspace.act(&run,BrowserAction::click(reply)).await.map_err(|error|error.to_string())?;
        workspace.finish_run(&run).await.map_err(|error|error.to_string())?;
        if !context_menus_enabled(&popup_view).await? {return Err("Popup input remained locked after the run".into())}
        let delivered=evaluate(&view,r#"new Promise(resolve=>{
            const ready=()=>popupMessages.some(message=>message.sameProxy && message.data.interaction && message.data.trusted);
            if(ready()){resolve(true);return}
            const finish=value=>{clearTimeout(timer);removeEventListener('message',listener);resolve(value)};
            const listener=()=>{if(ready())finish(true)};
            const timer=setTimeout(()=>finish(false),2000);addEventListener('message',listener);
        })"#).await?;
        if delivered!=true {return Err(format!("Managed popup interaction did not reach its real opener: {}",evaluate(&view,"popupMessages").await?))}
        let source_target=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing user popup runtime")?.tabs
            .into_iter().find(|tab|tab.target.tab_id==source).ok_or("Missing user opener")?.target;
        workspace.user_command(BrowserTabCommand::Activate {target:source_target.clone()}).await.map_err(|error|error.to_string())?;
        native_fixture_click(&view,"#unique").await?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        let user_popup=loop {
            let snapshot=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing user popup state")?;
            if snapshot.tabs.len()==3 {
                // Candidate publication retains cleanup ownership and quota;
                // it precedes native initialization and activation.
                if let Some(id)=snapshot.active_tab_id.as_ref() {
                    if id!=&source && id!=&popup.target.tab_id {break id.clone();}
                }
            }
            if tokio::time::Instant::now()>=deadline {return Err(format!("User-ready popup was not admitted and activated: {snapshot:?}"))}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        let user_view=app.get_webview(&user_popup).ok_or("Missing user popup view")?;
        if !context_menus_enabled(&user_view).await? {return Err("User-ready popup incorrectly stayed Agent-locked".into())}
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {
            let snapshot=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing loading popup state")?;
            if snapshot.tabs.iter().any(|tab|tab.target.tab_id==user_popup && tab.lifecycle==BrowserTabLifecycle::Ready && tab.url.ends_with("/popup-child")) {break}
            if tokio::time::Instant::now()>=deadline {return Err("Self-close popup did not load".into())}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let closing_click=native_fixture_click(&user_view,"#close").await;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {
            let snapshot=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing page-close state")?;
            if snapshot.tabs.len()==2 && !snapshot.tabs.iter().any(|tab|tab.target.tab_id==user_popup)
                && app.get_webview(&user_popup).is_none() {break}
            if tokio::time::Instant::now()>=deadline {
                let page=evaluate(&user_view,"({clicked:window.closeClickTrusted,method:typeof window.close,closed:window.closed})").await;
                return Err(format!("Page self-close did not remove its native tab: click={closing_click:?},page={page:?},state={snapshot:?}"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        if app.get_window("main").is_none() || evaluate(&view,"popupNonce").await?!=nonce {
            return Err("Closing a popup affected its opener or main window".into());
        }
        let closing_run=workspace.begin_run().await.map_err(|error|error.to_string())?;
        let observed=workspace.observe(&closing_run,Some(popup.target.tab_id.clone())).await.map_err(|error|error.to_string())?;
        let close_ref=observed.elements.iter().find(|element|element.name=="Close this popup").ok_or("Missing Agent popup close control")?.reference.clone();
        workspace.act(&closing_run,BrowserAction::click(close_ref)).await.map_err(|error|format!("Agent popup close: {error}"))?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {
            let snapshot=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing Agent page-close state")?;
            if snapshot.tabs.len()==1 && snapshot.active_tab_id.as_deref()==Some(source.as_str()) && app.get_webview(&popup.target.tab_id).is_none() {break}
            if tokio::time::Instant::now()>=deadline {return Err("Agent self-close did not finish native tab cleanup".into())}
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        workspace.finish_run(&closing_run).await.map_err(|error|error.to_string())?;
        workspace.user_command(BrowserTabCommand::Activate {target:source_target.clone()}).await.map_err(|error|error.to_string())?;
        native_fixture_click(&view,"#open").await?;
        let reopened=wait_workspace_page(&workspace,&url.replace("/popup-source","/popup-child")).await?;
        if reopened.tab_id==popup.target.tab_id || evaluate(&view,"popupReused").await?!=false {
            return Err("Reopening a closed named window reused a destroyed WindowProxy".into());
        }
        for _ in 2..8 {workspace.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(|error|error.to_string())?;}
        workspace.user_command(BrowserTabCommand::Activate {target:source_target}).await.map_err(|error|error.to_string())?;
        let before_labels:std::collections::BTreeSet<_>=app.get_window("main").unwrap().webviews().into_iter().map(|view|view.label().to_owned()).collect();
        // Existing named windows do not allocate another tab and must remain
        // usable at the quota. Exercise the real button through native input.
        native_fixture_click(&view,"#open").await?;
        let snapshot=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing named popup quota state")?;
        let labels:std::collections::BTreeSet<_>=app.get_window("main").unwrap().webviews().into_iter().map(|view|view.label().to_owned()).collect();
        if snapshot.tabs.len()!=8 || before_labels!=labels || evaluate(&view,"popupReused").await?!=true {
            return Err("Existing named popup could not be reused at the tab quota".into());
        }
        native_fixture_click(&view,"#unique").await?;
        let snapshot=workspace.snapshot().await.map_err(|error|error.to_string())?.runtime.ok_or("Missing quota state")?;
        let after_labels:std::collections::BTreeSet<_>=app.get_window("main").unwrap().webviews().into_iter().map(|view|view.label().to_owned()).collect();
        if snapshot.tabs.len()!=8 || snapshot.active_tab_id.as_deref()!=Some(source.as_str()) || before_labels!=after_labels {
            return Err("Popup bypassed the runtime tab quota".into());
        }
        Ok(serde_json::json!({"same_workspace_tab":true,"active_popup":true,"native_opener":proof,"agent_input_locked":true,
            "agent_interaction":true,"terminal_unlock":true,"user_ready_popup":true,"named_popup_same_proxy_and_tab":true,"closed_name_gets_new_proxy":true,"named_reuse_at_quota":true,"tab_quota_enforced":true,"unscoped_agent_popup_rejected":true,"page_self_close":true,"self_close_preserves_main_window":true,"agent_page_self_close":true,"spa_url_and_history_events":true,"native_permission_denied":true,"native_diagnostics":true,"diagnostics_no_getter_evaluation":true}))
    }.await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match service.close(&key).await {
            Ok(()) => break,
            Err(error) if tokio::time::Instant::now() >= deadline => {
                return Err(format!("Managed popup cleanup: {error}"));
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
        }
    }
    result
}

#[cfg(windows)]
async fn wait_navigation_metadata(
    workspace: &std::sync::Arc<nomifun_browser_platform::workspace::BrowserResource>,
    id: &str,
    url: &str,
    back: bool,
    forward: bool,
) -> Result<nomifun_browser_platform::runtime::BrowserTabTarget, String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let snapshot = workspace
            .snapshot()
            .await
            .map_err(|error| error.to_string())?
            .runtime
            .ok_or("Missing navigation metadata runtime")?;
        if let Some(tab) = snapshot.tabs.iter().find(|tab| {
            tab.target.tab_id == id
                && tab.url == url
                && tab.can_go_back == back
                && tab.can_go_forward == forward
        }) {
            return Ok(tab.target.clone());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "Native navigation metadata did not update: {snapshot:?}"
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[cfg(windows)]
async fn native_fixture_click(view: &tauri::Webview, selector: &str) -> Result<(), String> {
    let point=evaluate(view,&format!("(()=>{{const r=document.querySelector({selector:?}).getBoundingClientRect();return {{x:r.x+r.width/2,y:r.y+r.height/2}}}})()")).await?;
    for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
        windows::protocol_call(view,"Input.dispatchMouseEvent",serde_json::json!({"type":kind,"x":point["x"],"y":point["y"],
            "button":if kind=="mouseMoved" {"none"} else {"left"},"buttons":if kind=="mousePressed" {1} else {0},"clickCount":1})).await?;
    }
    Ok(())
}

#[cfg(windows)]
async fn native_bounds(view: &tauri::Webview) -> Result<[i32; 4], String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let mut bounds = ::windows::Win32::Foundation::RECT::default();
        let result = unsafe { platform.controller().Bounds(&mut bounds) }
            .map(|()| [bounds.left, bounds.top, bounds.right, bounds.bottom])
            .map_err(|_| "Native browser bounds are unavailable".to_owned());
        let _ = tx.send(result);
    })
    .map_err(|error| error.to_string())?;
    rx.await.map_err(|error| error.to_string())?
}

#[cfg(windows)]
async fn native_visible(view: &tauri::Webview) -> Result<bool, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let mut visible = ::windows::core::BOOL::default();
        let result = unsafe { platform.controller().IsVisible(&mut visible) }
            .map(|()| visible.as_bool())
            .map_err(|_| "Native visibility is unavailable".to_owned());
        let _ = tx.send(result);
    })
    .map_err(|error| error.to_string())?;
    rx.await.map_err(|error| error.to_string())?
}

#[cfg(windows)]
async fn verify_selections(
    workspace: &std::sync::Arc<nomifun_browser_platform::workspace::BrowserResource>,
    run: &nomifun_browser_platform::run_guard::BrowserRunGuard,
    tab_id: &str,
    view: &tauri::Webview,
) -> Result<serde_json::Value, String> {
    use nomifun_browser_platform::runtime::{BrowserAction, WorkspaceError};
    for forbidden in ["不可选择", "组内禁用", "隐藏选项", "重名", "不存在"] {
        let observation = workspace
            .observe(run, Some(tab_id.into()))
            .await
            .map_err(|e| e.to_string())?;
        let element = observation
            .elements
            .iter()
            .find(|el| el.name == "单选测试" && el.role == "combobox")
            .ok_or("Single select absent from native observation")?
            .reference
            .clone();
        if !matches!(
            workspace
                .act(
                    run,
                    BrowserAction::Select {
                        element,
                        labels: vec![forbidden.into()]
                    }
                )
                .await,
            Err(WorkspaceError::NotActionable)
        ) {
            return Err(format!("Invalid select option was accepted: {forbidden}"));
        }
    }
    if evaluate(view, "smoke.events.filter(e=>e.target==='single').length").await? != 0 {
        return Err("Rejected select issued browser input".into());
    }
    for (name, id, labels) in [
        ("单选测试", "single", vec!["Café 中文"]),
        ("单选测试", "single", vec!["Café 中文"]),
        ("多选测试", "multi", vec!["Alpha", "Omega"]),
        ("多选测试", "multi", vec![]),
    ] {
        let observation = workspace
            .observe(run, Some(tab_id.into()))
            .await
            .map_err(|e| e.to_string())?;
        let element = observation
            .elements
            .iter()
            .find(|el| el.name == name && matches!(el.role.as_str(), "combobox" | "listbox"))
            .ok_or_else(|| format!("Select {name} absent from native observation"))?
            .reference
            .clone();
        let action = BrowserAction::Select {
            element,
            labels: labels.iter().map(|label| (*label).into()).collect(),
        };
        workspace
            .act(run, action)
            .await
            .map_err(|e| format!("Select {name} failed: {e}"))?;
        let actual = evaluate(
            view,
            &format!(
                "Array.from(document.getElementById('{id}').selectedOptions, option=>option.label)"
            ),
        )
        .await?;
        if actual != serde_json::json!(labels) {
            return Err(format!(
                "Native selection mismatch: expected {labels:?}, received {actual}"
            ));
        }
    }
    let changing = workspace
        .observe(run, Some(tab_id.into()))
        .await
        .map_err(|e| e.to_string())?;
    let element = changing
        .elements
        .iter()
        .find(|el| el.name == "动态单选")
        .ok_or("Dynamic select missing from observation")?
        .reference
        .clone();
    if !matches!(
        workspace
            .act(
                run,
                BrowserAction::Select {
                    element,
                    labels: vec!["Omega".into()]
                }
            )
            .await,
        Err(WorkspaceError::ActionInterrupted)
    ) {
        return Err("Selection continued after the page replaced its options".into());
    }
    let mutation = evaluate(view,"({changes:smoke.selectMutations,keys:smoke.events.filter(e=>e.target==='changing'&&e.type==='keydown').length})").await?;
    if mutation["changes"] != 1 || mutation["keys"] != 1 {
        return Err(format!(
            "Selection replayed input after options changed: {mutation}"
        ));
    }
    let blocked = workspace
        .observe(run, Some(tab_id.into()))
        .await
        .map_err(|e| e.to_string())?;
    let element = blocked
        .elements
        .iter()
        .find(|el| el.name == "拦截方向键")
        .ok_or("Keyboard-blocking select missing")?
        .reference
        .clone();
    if !matches!(
        workspace
            .act(
                run,
                BrowserAction::Select {
                    element,
                    labels: vec!["Alpha".into(), "Omega".into()]
                }
            )
            .await,
        Err(WorkspaceError::ActionInterrupted)
    ) {
        return Err("Canceled native arrow key did not stop selection".into());
    }
    let blocked_state = evaluate(
        view,
        "Array.from(document.getElementById('blocked').selectedOptions,option=>option.label)",
    )
    .await?;
    if blocked_state != serde_json::json!(["Alpha", "Beta"]) {
        return Err(format!(
            "Canceled arrow key toggled the wrong option: {blocked_state}"
        ));
    }
    if evaluate(
        view,
        "smoke.events.filter(e=>e.target==='single'&&e.type==='keydown').length",
    )
    .await?
        != 2
    {
        return Err("Selecting an already-selected option repeated keyboard input".into());
    }
    evaluate(
        view,
        "document.getElementById('multi').style.writingMode='vertical-rl'",
    )
    .await?;
    let vertical = workspace
        .observe(run, Some(tab_id.into()))
        .await
        .map_err(|e| e.to_string())?;
    let element = vertical
        .elements
        .iter()
        .find(|el| el.name == "多选测试")
        .ok_or("Vertical select missing")?
        .reference
        .clone();
    workspace
        .act(
            run,
            BrowserAction::Select {
                element,
                labels: vec!["Omega".into()],
            },
        )
        .await
        .map_err(|e| format!("Vertical select: {e}"))?;
    if evaluate(
        view,
        "Array.from(document.getElementById('multi').selectedOptions,option=>option.label)",
    )
    .await?
        != serde_json::json!(["Omega"])
    {
        return Err("Vertical native listbox selected the wrong option".into());
    }
    let events = evaluate(
        view,
        "smoke.events.filter(e=>e.target==='single'||e.target==='multi')",
    )
    .await?;
    let list = events.as_array().ok_or("Missing select events")?;
    if list.iter().any(|event| event["trusted"] != true) {
        return Err("Synthetic event reached select input evidence".into());
    }
    for id in ["single", "multi"] {
        for kind in ["keydown", "keyup", "input", "change"] {
            if !list
                .iter()
                .any(|event| event["target"] == id && event["type"] == kind)
            {
                return Err(format!("Native select {id} omitted {kind}: {events}"));
            }
        }
    }
    Ok(
        serde_json::json!({"single":true,"multi":true,"clear_multi":true,"vertical_listbox":true,"idempotent_single":true,"canceled_arrow_stops_input":true,"invalid_options_rejected":true,"option_mutation_stops_input":mutation,"events":events}),
    )
}

#[cfg(windows)]
async fn wait_workspace_page(
    workspace: &nomifun_browser_platform::workspace::BrowserResource,
    url: &str,
) -> Result<nomifun_browser_platform::runtime::BrowserTabTarget, String> {
    loop {
        let snapshot = workspace
            .snapshot()
            .await
            .map_err(|error| error.to_string())?;
        let runtime = snapshot.runtime.ok_or("Missing native runtime")?;
        if let Some(tab) = runtime.tabs.iter().find(|tab| {
            tab.url == url
                && tab.lifecycle == nomifun_browser_platform::runtime::BrowserTabLifecycle::Ready
        }) {
            return Ok(tab.target.clone());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
