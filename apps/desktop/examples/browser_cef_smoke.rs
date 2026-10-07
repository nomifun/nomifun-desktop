//! Real macOS CEF child view inside Tauri's native window and message loop.
//! Run the packaged example; a missing CEF bundle is a failure, never a skip.
#[cfg(target_os = "macos")]
#[path = "../src/browser_surface/macos/mod.rs"]
mod macos;
#[cfg(target_os = "macos")]
use macos::native;
#[cfg(target_os = "macos")]
#[path = "../src/browser_surface/automation.rs"]
mod automation;
#[cfg(target_os = "macos")]
#[path = "support/browser_frame_input.rs"]
mod browser_frame_input;
#[cfg(target_os = "macos")]
#[path = "support/browser_upload_frames.rs"]
mod browser_upload_frames;
#[cfg(target_os = "macos")]
#[path = "support/browser_window_reopen.rs"]
mod browser_window_reopen;
#[cfg(not(target_os = "macos"))]
fn main() { eprintln!("This native fixture requires macOS. Windows uses browser_workspace_smoke."); std::process::exit(2); }

#[cfg(target_os = "macos")]
fn main() {
    use std::{io::{Read, Write}, sync::Arc};
    use nomifun_browser_macos::engine::{Engine, Paths, ParentView};
    use nomifun_browser_platform::runtime::BrowserSurfaceBounds;
    use tauri::Manager;
    let report = std::env::var_os("NOMIFUN_CEF_REPORT").map(std::path::PathBuf::from)
        .unwrap_or_else(|| { eprintln!("NOMIFUN_CEF_REPORT is required"); std::process::exit(2) });
    let soak_only = std::env::var_os("NOMIFUN_CEF_SOAK_ONLY").is_some();
    let window_reopen_only = std::env::var_os("NOMIFUN_CEF_WINDOW_REOPEN_ONLY").is_some();
    let context_shutdown_only = std::env::var_os("NOMIFUN_CEF_CONTEXT_SHUTDOWN_ONLY").is_some();
    let sqlite_compatibility_only = std::env::var_os("NOMIFUN_CEF_SQLITE_COMPATIBILITY_ONLY").is_some();
    let cold_navigation_only = std::env::var_os("NOMIFUN_CEF_COLD_NAVIGATION_ONLY").is_some();
    let old_host_allocations = sqlite_compatibility_only.then(|| (0..2000)
        .map(|index| Arc::new(std::sync::Mutex::new(format!("host allocation before preload {index}"))))
        .collect::<Vec<_>>());
    let bundle = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    let framework = bundle.join("Contents/Frameworks/Chromium Embedded Framework.framework");
    // SAFETY: no fixture HTTP, Tauri or Tokio worker exists at this point.
    if let Err(error) = unsafe { Engine::preload_framework(&framework) } {
        let _ = std::fs::write(&report, serde_json::to_vec_pretty(&serde_json::json!({"passed":false,"preload_error":error})).unwrap());
        std::process::exit(2);
    }
    drop(old_host_allocations);
    std::fs::write(report.with_extension("pid"), std::process::id().to_string()).expect("fixture PID receipt");
    if sqlite_compatibility_only {
        // This branch never builds Tauri or calls Engine::initialize. The
        // temporary database and all workers are created only after preload.
        let result = verify_sqlite_compatibility();
        let (value, passed) = match result {
            Ok(value) => (value, true),
            Err(error) => (serde_json::json!({"passed":false,"error":error}), false),
        };
        let written = std::fs::write(&report, serde_json::to_vec_pretty(&value).unwrap()).is_ok();
        std::process::exit(if passed && written { 0 } else { 1 });
    }
    let root = tempfile::Builder::new().prefix("nomi-cef-smoke-").tempdir().expect("disposable CEF profile");
    let data = root.path().to_path_buf();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let fixture_started = std::time::Instant::now();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break; };
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut request = [0u8; 4096];
                let Ok(count) = stream.read(&mut request) else { return; };
                let request = String::from_utf8_lossy(&request[..count]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/");
                #[cfg(debug_assertions)]
                eprintln!("CEF_SMOKE_HTTP received=true fixture_elapsed_ms={} route={}", fixture_started.elapsed().as_millis(), match path {
                    "/" => "root", "/download-file" => "download", "/download-slow-file" => "download_slow",
                    "/upload-frames" => "upload_frames", "/popup-source" => "popup_source", "/popup-child" => "popup_child",
                    _ => "other_fixture_route",
                });
                if path == "/download-file" {
                    let body = "Native CEF download 中文\n";
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Disposition: attachment; filename=cef-download.txt\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                    return;
                }
                if path == "/download-slow-file" {
                    const TOTAL: usize = 32 * 1024 * 1024;
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=cef-cancel.bin\r\nContent-Length: {TOTAL}\r\nConnection: close\r\n\r\n");
                    let block = [0x5au8; 64 * 1024];
                    for _ in 0..TOTAL / block.len() {
                        if stream.write_all(&block).is_err() { break; }
                        let _ = stream.flush();
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    return;
                }
                let body = if path == "/upload-frames" {
                    include_str!("fixtures/browser_upload_frames.html").replace("__PORT__", &address.port().to_string())
                } else if matches!(path,"/upload-child-cross" | "/upload-child-same") {
                    include_str!("fixtures/browser_upload_child.html").replace("__LEVEL__", if path=="/upload-child-cross" { "Cross" } else { "Same" })
                } else if path == "/frame-sessions" {
                    include_str!("fixtures/browser_frames.html").replace("__PORT__", &address.port().to_string())
                } else if matches!(path, "/frame-child" | "/frame-nested" | "/frame-same" | "/frame-same-nested") {
                    let (level, nested) = match path {
                        "/frame-child" => ("Cross-site", format!("<iframe src=\"http://127.0.0.1:{}/frame-nested\"></iframe>", address.port())),
                        "/frame-nested" => ("Nested", String::new()),
                        "/frame-same-nested" => ("Same-nested", String::new()),
                        _ => ("Same-process", String::new()),
                    };
                    include_str!("fixtures/browser_frame_content.html").replace("__LEVEL__",level).replace("__NESTED__", &nested)
                } else if path == "/popup-source" {
                    include_str!("fixtures/browser_popup.html").to_owned()
                } else if path == "/popup-child" {
                    include_str!("fixtures/browser_popup_child.html").to_owned()
                } else if path == "/download-source" {
                    "<!doctype html><meta charset=utf-8><title>CEF download</title><a href='/download-file'>Download local fixture</a>".to_owned()
                } else if path == "/user-downloads" {
                    "<!doctype html><meta charset=utf-8><title>CEF user downloads</title><a id='download' href='/download-file'>Download complete</a><br><a id='cancel' href='/download-slow-file'>Download cancel</a>".to_owned()
                } else if path == "/user-files" {
                    include_str!("fixtures/browser_user_files.html").replace("__PORT__", &address.port().to_string())
                } else { include_str!("fixtures/browser_workspace.html")
                    .replace("<div id=\"scroller\">", "<div id=\"scroller\" role=\"region\" aria-label=\"滚动区域\">")
                    .replace("<div id=\"drag\">", "<div id=\"drag\" role=\"button\" aria-label=\"拖拽区域\">") };
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            });
        }
    });
    let helper = bundle.join("Contents/Frameworks/NomiCEFSmoke Helper.app/Contents/MacOS/NomiCEFSmoke Helper");
    let paths = Paths { framework, helper, main_bundle: bundle, data_root: data.clone() };
    let engine_slot = Arc::new(std::sync::OnceLock::<Arc<Engine>>::new());
    let setup_engine = engine_slot.clone();
    let failure_report = report.clone();
    let app = tauri::Builder::default().setup(move |app| {
        tauri::window::WindowBuilder::new(app, "main").title("NomiFun — macOS CEF native conformance")
            .inner_size(1100.0, 720.0).min_inner_size(880.0, 600.0).build()?;
        if window_reopen_only {
            let guard = tauri::window::WindowBuilder::new(app, "lifecycle-guard")
                .title("NomiFun native lifecycle guard")
                .inner_size(1.0, 1.0)
                .build()?;
            guard.hide()?;
        }
        let engine = setup_engine.get().expect("CEF initialized before app run").clone();
        let handle = app.handle().clone();
        let parent_handle = handle.clone();
        let parent: Arc<ParentView> = Arc::new(move || {
            let window = parent_handle.get_window("main").ok_or("Tauri main window is gone")?;
            let raw = window.ns_window().map_err(|_| "Tauri native window is unavailable")?;
            let window = unsafe { raw.cast::<objc2_app_kit::NSWindow>().as_ref() }.ok_or("Native window is null")?;
            window.contentView().ok_or_else(|| "Native content view is unavailable".into())
        });
        let watchdog = handle.clone();
        tauri::async_runtime::spawn(async move {
            // The storage matrix intentionally permits a 30-second bounded
            // CEF clear operation after the input/frame/runtime suites. A
            // 90-second whole-process watchdog became shorter than the valid
            // aggregate on current macOS/CEF builds and killed owned helpers
            // while the final proof still held cleanup authority.
            tokio::time::sleep(std::time::Duration::from_secs(180)).await;
            eprintln!("CEF_SMOKE_FAIL native operation or shutdown timed out");
            watchdog.exit(2);
        });
        tauri::async_runtime::spawn(async move {
            let mut retained_shutdown_context = None;
            let result = async {
                if cold_navigation_only {
                    return verify_cold_host_navigation(&engine, &handle, &data, &format!("http://{address}/browser_workspace.html")).await;
                }
                if context_shutdown_only {
                    retained_shutdown_context = Some(engine.create_context(Some(data.join("shutdown-context"))).await?);
                    eprintln!("CEF_SMOKE_PHASE shutdown_context_created_without_page");
                    return Ok::<_, String>(serde_json::json!({"checks":{"request_context_created":true},"passed":true}));
                }
                if window_reopen_only {
                    return browser_window_reopen::verify(
                        &engine,
                        &handle,
                        &format!("http://{address}"),
                    ).await;
                }
                if soak_only {
                    return verify_runtime_soak(
                        &engine,
                        &handle,
                        &format!("http://{address}"),
                    ).await;
                }
                let context = engine.create_context(None).await?;
                eprintln!("CEF_SMOKE_PHASE context_ready");
                let page = engine.create_page(parent.clone(), context.clone()).await?;
                eprintln!("CEF_SMOKE_PHASE native_child_created");
                page.set_surface(BrowserSurfaceBounds { x: 20., y: 60., width: 1060., height: 620. }, true, Default::default()).await?;
                page.protocol.call(None, "Page.enable", serde_json::json!({})).await?;
                eprintln!("CEF_SMOKE_PHASE protocol_ready");
                page.protocol.call(None, "Page.navigate", serde_json::json!({"url":format!("http://{address}/browser_workspace.html")})).await?;
                eprintln!("CEF_SMOKE_PHASE navigation_completed");
                for _ in 0..100 {
                    if evaluate(&page, "typeof smoke !== 'undefined'").await? == true { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                if evaluate(&page, "typeof smoke !== 'undefined'").await? != true { return Err("Native page did not load".to_owned()); }
                let view = native::View::new(page.clone());
                let mut driver = automation::TabAutomation::default();
                let target = nomifun_browser_platform::runtime::BrowserTabTarget { tab_id: "cef-smoke".into(), runtime_generation: 1, document_generation: 1 };
                let cancel = tokio_util::sync::CancellationToken::new();
                driver.activate_for_agent(&view, &cancel).await.map_err(|e|e.to_string())?;
                let observed = driver.observe(&view, target.clone(), &cancel).await.map_err(|e|e.to_string())?;
                let field = observed.elements.iter().find(|e|e.name=="输入内容" && e.role=="textbox").ok_or("Semantic field reference missing")?.reference.clone();
                driver.act(&view, nomifun_browser_platform::runtime::BrowserAction::Type { element: field, text: "replace this text".into() }, &cancel).await.map_err(|e|e.to_string())?;
                let observed = driver.observe(&view, target.clone(), &cancel).await.map_err(|e|e.to_string())?;
                let field = observed.elements.iter().find(|e|e.name=="输入内容" && e.role=="textbox").ok_or("Semantic replacement field missing")?.reference.clone();
                driver.act(&view, nomifun_browser_platform::runtime::BrowserAction::Type { element: field, text: "CEF 中文输入".into() }, &cancel).await.map_err(|e|e.to_string())?;
                let focused = evaluate(&page, "document.activeElement.id").await?;
                let observed = driver.observe(&view, target.clone(), &cancel).await.map_err(|e|e.to_string())?;
                let submit = observed.elements.iter().find(|e|e.name=="验证点击" && e.role=="button").ok_or("Semantic button reference missing")?.reference.clone();
                driver.act(&view, nomifun_browser_platform::runtime::BrowserAction::click(submit), &cancel).await.map_err(|e|e.to_string())?;
                let observed = driver.observe(&view, target.clone(), &cancel).await.map_err(|e|e.to_string())?;
                let scroll = observed.elements.iter().find(|e|e.name=="滚动区域").ok_or("Semantic scroll reference missing")?.reference.clone();
                driver.act(&view, nomifun_browser_platform::runtime::BrowserAction::Scroll { element:scroll, delta_x:0., delta_y:150. }, &cancel).await.map_err(|e|e.to_string())?;
                let observed = driver.observe(&view, target, &cancel).await.map_err(|e|e.to_string())?;
                let from = observed.elements.iter().find(|e|e.name=="拖拽区域").ok_or("Semantic drag reference missing")?.reference.clone();
                let to = observed.elements.iter().find(|e|e.name=="验证点击" && e.role=="button").ok_or("Semantic drag destination missing")?.reference.clone();
                driver.act(&view, nomifun_browser_platform::runtime::BrowserAction::Drag { from, to }, &cancel).await.map_err(|e|e.to_string())?;
                let state = evaluate(&page, "({value:field.value,clicks:smoke.clicks,events:smoke.events,scroll:scroller.scrollTop,capturedMoves:smoke.capturedMoves})").await?;
                let events = state["events"].as_array().ok_or("Missing native event evidence")?;
                let downs: Vec<_> = events.iter().filter(|e| e["type"] == "pointerdown").collect();
                let mut checks = serde_json::json!({
                    "focus":focused=="field", "unicode_text":state["value"]=="CEF 中文输入", "click":state["clicks"]==1,
                    "trusted_input":!events.is_empty() && events.iter().all(|e|e["trusted"]==true),
                    "buttons":!downs.is_empty() && downs.iter().all(|e|e["buttons"]==1),
                    "drag":state["capturedMoves"].as_u64().unwrap_or(0)>0,
                    "wheel":state["scroll"].as_u64().unwrap_or(0)>0,
                    "input_gate_locked":page.input_locked(),
                });
                driver.settle_agent(&view).await.map_err(|e|e.to_string())?;
                drop(driver);
                let submit = point(&page, "submit").await?;
                let before = page.blocked_input_count();
                queue_user_click(&handle, submit).await?;
                for _ in 0..100 {
                    if page.blocked_input_count() >= before + 2 { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                checks["local_user_input_gate"] = (page.blocked_input_count() >= before + 2 && evaluate(&page, "smoke.clicks").await? == 1).into();
                page.set_input_locked(false).await?;
                queue_user_click(&handle, submit).await?;
                for _ in 0..100 {
                    if evaluate(&page, "smoke.clicks").await? == 2 { break; }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                checks["user_input_restored"] = (evaluate(&page, "smoke.clicks").await? == 2).into();
                verify_dialogs(&page, &mut checks).await?;
                page.force_close().await?;
                eprintln!("CEF_SMOKE_PHASE primary_page_closed");
                let frames = engine.create_page(parent.clone(), context.clone()).await?;
                frames.set_surface(BrowserSurfaceBounds { x:20., y:60., width:1060., height:620. }, true, Default::default()).await?;
                browser_frame_input::verify_frame_input(&native::View::new(frames.clone()), &format!("http://{address}/frame-sessions")).await?;
                checks["native_frame_input_and_geometry"] = true.into();
                frames.force_close().await?;
                eprintln!("CEF_SMOKE_PHASE frame_input_settled");
                let uploads = engine.create_page(parent.clone(), context).await?;
                uploads.set_surface(BrowserSurfaceBounds { x:20., y:60., width:1060., height:620. }, true, Default::default()).await?;
                browser_upload_frames::verify(&native::View::new(uploads.clone()), &format!("http://{address}/upload-frames")).await?;
                checks["native_frame_uploads_and_stale_chooser"] = true.into();
                uploads.force_close().await?;
                eprintln!("CEF_SMOKE_PHASE frame_uploads_settled");
                verify_user_file_picker(&engine, &handle, parent.clone(), &data, &format!("http://{address}/user-files"), &mut checks).await?;
                verify_user_downloads(&engine, &handle, parent.clone(), &format!("http://{address}/user-downloads"), &mut checks).await?;
                verify_permissions(&engine, &handle, parent.clone(), &format!("http://{address}/popup-source"), &mut checks).await?;
                verify_runtime(&engine, &handle, &format!("http://{address}"), &mut checks).await?;
                verify_renderer_crash(&engine, parent.clone(), &format!("http://{address}/popup-source"), &mut checks).await?;
                verify_storage(&engine, parent, &data, &format!("http://{address}"), &mut checks).await?;
                Ok::<_, String>(serde_json::json!({"checks":checks,"page":state,"passed":checks.as_object().unwrap().values().all(|v|v==true)}))
            }.await;
            eprintln!("CEF_SMOKE_PHASE shutdown_begin");
            let shutdown = engine.shutdown().await;
            eprintln!("CEF_SMOKE_PHASE shutdown_returned");
            drop(retained_shutdown_context);
            let result = match (result, shutdown) {
                (Ok(mut value), Ok(())) => { value["shutdown_complete"] = true.into(); value },
                (result, shutdown) => serde_json::json!({"passed":false,"error":result.err(),"shutdown_error":shutdown.err()}),
            };
            let passed = result["passed"] == true;
            let written = std::fs::write(&report, serde_json::to_vec_pretty(&result).unwrap()).is_ok();
            handle.exit(if passed && written { 0 } else { 1 });
        });
        Ok(())
    }).build(tauri::generate_context!()).expect("Tauri CEF fixture setup");
    // Tao's NSApplication subclass now exists, but its run loop has not started.
    let runtime=tauri::async_runtime::handle();
    let _runtime=runtime.inner().enter();
    match Engine::initialize(paths) {
        Ok(engine) => { let _ = engine_slot.set(engine); }
        Err(error) => {
            let _ = std::fs::write(failure_report, serde_json::to_vec_pretty(&serde_json::json!({"passed":false,"setup_error":error})).unwrap());
            std::process::exit(2);
        }
    }
    let code = app.run_return(|_, _| {});
    drop(root);
    std::process::exit(code);
}

#[cfg(target_os = "macos")]
async fn verify_cold_host_navigation(
    engine: &std::sync::Arc<nomifun_browser_macos::engine::Engine>,
    app: &tauri::AppHandle,
    data: &std::path::Path,
    url: &str,
) -> Result<serde_json::Value, String> {
    use nomifun_browser_platform::{runtime::*, run_guard::{BrowserInputState, BrowserRunCoordinator}};
    let host = macos::host::DesktopBrowserHost::new(app.clone(), engine.clone());
    let ephemeral = match std::env::var("NOMIFUN_CEF_COLD_NAVIGATION_PROFILE").as_deref() {
        Ok("ephemeral") => true,
        Err(_) | Ok("persistent") => false,
        _ => return Err("cold fixture profile must be persistent or ephemeral".into()),
    };
    let runtime = host.create(CreateBrowserRuntime {
        key: BrowserResourceKey { principal_id: "fixture-user".into(), agent_session_id: "cold-host".into(), resource_binding_id: "managed-browser".into() },
        runtime_generation: 1, profile: if ephemeral { BrowserProfile::Ephemeral } else { BrowserProfile::Persistent(data.join("cold-host-profile")) }, user_input_enabled: true,
    }).await.map_err(|error| error.to_string())?;
    runtime.surface().ok_or("cold fixture native surface is missing")?
        .set_surface(BrowserSurfaceBounds { x:20., y:60., width:1060., height:620. }, true, Default::default())
        .await.map_err(|error| error.to_string())?;
    let created = std::time::Instant::now();
    let result = async {
        // This is the first page in a fresh native process and uses the real
        // product host, including registered ownership and bootstrap barrier.
        runtime.execute(BrowserTabCommand::Create { url: url.into() }, Default::default())
            .await.map_err(|error| error.to_string())?;
        let create_elapsed_ms = created.elapsed().as_millis();
        let mut changes = runtime.changes().ok_or("cold fixture native metadata is missing")?;
        let target = tokio::time::timeout(std::time::Duration::from_secs(8), async {
            loop {
                let snapshot = runtime.snapshot().await.map_err(|error| error.to_string())?;
                if let Some(tab) = snapshot.tabs.iter().find(|tab| tab.url == url && tab.lifecycle == BrowserTabLifecycle::Ready) {
                    return Ok::<_, String>(tab.target.clone());
                }
                changes.changed().await.map_err(|_| "cold fixture metadata owner was lost")?;
            }
        }).await.map_err(|_| "cold navigation did not produce the actual ready document")??;
        let coordinator = BrowserRunCoordinator::new(runtime.clone());
        let run = coordinator.begin().await.map_err(|error| error.to_string())?;
        run.require_explicit_finish();
        let observation = { let runtime = runtime.clone(); coordinator.agent_operation(&run, move |cancel| async move {
            Ok(runtime.automation().expect("real native host automation").observe(None, cancel).await)
        }).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())? };
        coordinator.settle(&run).await.map_err(|error| error.to_string())?;
        let locked = coordinator.snapshot().await.input_state == BrowserInputState::AgentRunning;
        coordinator.finish(&run).await.map_err(|error| error.to_string())?;
        let checks = serde_json::json!({
            "cold_first_navigation_ready":true,
            "navigation_uses_same_owned_page":observation.target == target,
            "protocol_survives_first_navigation":!observation.elements.is_empty(),
            "user_input_locked_until_finish":locked,
            "user_input_recovered":coordinator.snapshot().await.input_state == BrowserInputState::UserReady,
        });
        Ok::<_, String>(serde_json::json!({"passed":checks.as_object().unwrap().values().all(|value|value==true),"checks":checks,"create_elapsed_ms":create_elapsed_ms,"profile":if ephemeral { "ephemeral" } else { "persistent" }}))
    }.await;
    let closed = runtime.close().await.map_err(|error| error.to_string());
    match (result, closed) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (_, Err(error)) => Err(error),
    }
}

#[cfg(target_os = "macos")]
fn verify_sqlite_compatibility() -> Result<serde_json::Value, String> {
    use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
    use sqlx::{Row, sqlite::{SqliteConnectOptions, SqlitePoolOptions}};
    use nomifun_browser_macos::engine::Engine;
    const WORKERS: usize = 8;
    const PER_WORKER: usize = 5000;
    if Engine::initialized_in_process() { return Err("allocator fixture unexpectedly initialized CEF".into()); }
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(WORKERS)
        .enable_all().build().map_err(|error| error.to_string())?;
    let completed = Arc::new(AtomicUsize::new(0));
    let count = completed.clone();
    runtime.block_on(async move {
        let options: SqliteConnectOptions = "sqlite::memory:".parse().map_err(|error: sqlx::Error| error.to_string())?;
        let pool = SqlitePoolOptions::new().max_connections(WORKERS as u32).min_connections(WORKERS as u32)
            .connect_with(options).await.map_err(|error| error.to_string())?;
        sqlx::query("CREATE TABLE agent_sessions(native_contract_id TEXT,native_cursor_json TEXT,native_checkpoint_json TEXT)")
            .execute(&pool).await.map_err(|error| error.to_string())?;
        let mut tasks = tokio::task::JoinSet::new();
        for worker in 0..WORKERS {
            let pool = pool.clone();
            let count = count.clone();
            tasks.spawn(async move {
                // Retain one connection for each task so eight SQLite native
                // workers exercise their own lookaside pools concurrently.
                let mut connection = pool.acquire().await.map_err(|error| error.to_string())?;
                for iteration in 0..PER_WORKER {
                    let rows = sqlx::query("SELECT name FROM pragma_table_info(?)").bind("agent_sessions")
                        .fetch_all(&mut *connection).await.map_err(|error| error.to_string())?;
                    let columns = rows.iter().map(|row| row.try_get::<String, _>(0))
                        .collect::<Result<Vec<_>, _>>().map_err(|error| error.to_string())?;
                    if columns != ["native_contract_id", "native_cursor_json", "native_checkpoint_json"] {
                        return Err("SQLite schema projection was corrupted after preload".to_owned());
                    }
                    let memory = Arc::new(Mutex::new(format!("worker={worker} iteration={iteration} native_cursor_json")));
                    let (send, receive) = std::sync::mpsc::channel();
                    send.send(memory.clone()).map_err(|error| error.to_string())?;
                    let memory = receive.recv().map_err(|error| error.to_string())?;
                    if !memory.lock().map_err(|error| error.to_string())?.contains("native_cursor_json") {
                        return Err("host allocation content was corrupted".to_owned());
                    }
                    count.fetch_add(1, Ordering::Relaxed);
                }
                Ok::<_, String>(())
            });
        }
        let mut failure = None;
        while let Some(result) = tasks.join_next().await {
            if let Err(error) = result.map_err(|error| error.to_string()).and_then(|result| result) {
                failure = Some(error);
            }
        }
        pool.close().await;
        failure.map_or(Ok(()), Err)
    })?;
    drop(runtime);
    if completed.load(Ordering::Relaxed) != WORKERS * PER_WORKER || Engine::initialized_in_process() {
        return Err("CEF/SQLite compatibility fixture did not complete its exact scope".into());
    }
    Ok(serde_json::json!({
        "passed":true,"shutdown_complete":true,
        "scope":"early-library-preload-sqlite-compatibility",
        "sqlite_workers":WORKERS,"schema_queries":completed.load(Ordering::Relaxed),
        "cef_initialized":false,"helper_processes_started":0,
        "checks":{"preload_before_workers":true,"legacy_host_allocations_freed":true,
            "sqlite_schema_and_host_allocation_churn":true,"cef_engine_not_initialized":true}
    }))
}

#[cfg(target_os = "macos")]
async fn evaluate(page: &nomifun_browser_macos::engine::Page, expression: &str) -> Result<serde_json::Value, String> {
    let value = page.protocol.call(None, "Runtime.evaluate", serde_json::json!({"expression":expression,"returnByValue":true})).await?;
    if value.get("exceptionDetails").is_some() { return Err("Fixture evaluation failed".into()); }
    Ok(value["result"]["value"].clone())
}
#[cfg(target_os = "macos")]
async fn point(page: &nomifun_browser_macos::engine::Page, id: &str) -> Result<(f64,f64), String> {
    let p = evaluate(page, &format!("(()=>{{const r=document.getElementById('{id}').getBoundingClientRect();return [r.x+r.width/2,r.y+r.height/2]}})()")).await?;
    Ok((p[0].as_f64().ok_or("Missing element x")?,p[1].as_f64().ok_or("Missing element y")?))
}
#[cfg(target_os = "macos")]
async fn queue_user_click(handle: &tauri::AppHandle, point: (f64, f64)) -> Result<(), String> {
    use tauri::Manager;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSWindow};
    let (tx, rx) = tokio::sync::oneshot::channel();
    let app = handle.clone();
    handle.run_on_main_thread(move || {
        let result = (|| {
            let window = app.get_window("main").ok_or("Fixture window is gone")?;
            let raw = window.ns_window().map_err(|_| "Fixture native window is unavailable")?;
            let window = unsafe { raw.cast::<NSWindow>().as_ref() }.ok_or("Fixture native window is null")?;
            let view = window.contentView().ok_or("Fixture content view is gone")?;
            let position = objc2_foundation::NSPoint::new(20. + point.0, view.bounds().size.height - 60. - point.1);
            let application = NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
            for kind in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
                let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(kind, position, NSEventModifierFlags::empty(), objc2_foundation::NSProcessInfo::processInfo().systemUptime(), window.windowNumber(), None, 1, 1, if kind == NSEventType::LeftMouseDown { 1. } else { 0. }).ok_or("Fixture mouse event could not be created")?;
                application.postEvent_atStart(&event, false);
            }
            Ok::<_, &str>(())
        })().map_err(str::to_owned);
        let _ = tx.send(result);
    }).map_err(|_| "Fixture UI dispatch failed")?;
    rx.await.map_err(|_| "Fixture UI event queue failed")?
}

#[cfg(target_os = "macos")]
async fn await_dialog(page: &nomifun_browser_macos::engine::Page) -> Result<nomifun_browser_macos::engine::NativeDialog, String> {
    let mut changes = page.subscribe();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(dialog) = changes.borrow_and_update().dialog.clone() { return Ok(dialog); }
            changes.changed().await.map_err(|_| "Dialog fixture state closed".to_owned())?;
        }
    }).await.map_err(|_| "Native dialog was not offered".to_owned())?
}
#[cfg(target_os = "macos")]
async fn verify_dialogs(page: &std::sync::Arc<nomifun_browser_macos::engine::Page>, checks: &mut serde_json::Value) -> Result<(), String> {
    use tokio_util::sync::CancellationToken;
    page.set_input_locked(true).await?;
    page.set_dialog_draining(false).await?;
    let pending = { let page = page.clone(); tokio::spawn(async move { evaluate(&page, "window.dialogRuns=(window.dialogRuns||0)+1;prompt('CEF 提示','默认')").await }) };
    let dialog = await_dialog(page).await?;
    let invalid = page.reply_dialog("wrong-id".into(), dialog.document_generation, true, "错误".into(), CancellationToken::new()).await;
    let cancelled = CancellationToken::new(); cancelled.cancel();
    let rejected = page.reply_dialog(dialog.request_id.clone(), dialog.document_generation, true, "错误".into(), cancelled).await;
    checks["dialog_stale_and_cancel_rejected"] = (invalid.is_err() && rejected.is_err() && !pending.is_finished()).into();
    page.reply_dialog(dialog.request_id, dialog.document_generation, true, "中文回复".into(), CancellationToken::new()).await?;
    let result = pending.await.map_err(|_| "Prompt operation panicked")??;
    checks["native_prompt_reply"] = (result == "中文回复" && page.snapshot().dialog.is_none() && evaluate(page, "dialogRuns").await? == 1).into();
    let pending = { let page = page.clone(); tokio::spawn(async move { evaluate(&page, "confirm('Stop 必须取消原操作')").await }) };
    await_dialog(page).await?;
    page.set_dialog_draining(true).await?;
    let result = pending.await.map_err(|_| "Confirm operation panicked")??;
    checks["dialog_stop_drain"] = (result == false && page.snapshot().dialog.is_none() && !page.protocol.is_closed()).into();
    checks["draining_suppresses_new_dialogs"] = (evaluate(page, "confirm('仍在停止')").await? == false && page.snapshot().dialog.is_none()).into();
    eprintln!("CEF_SMOKE_PHASE native_dialogs_settled");
    Ok(())
}

#[cfg(target_os = "macos")]
async fn verify_user_file_picker(
    engine: &std::sync::Arc<nomifun_browser_macos::engine::Engine>,
    app: &tauri::AppHandle,
    parent: std::sync::Arc<nomifun_browser_macos::engine::ParentView>,
    initial_directory: &std::path::Path,
    url: &str,
    checks: &mut serde_json::Value,
) -> Result<(), String> {
    use std::sync::{Arc, atomic::AtomicBool};
    let context = engine.create_context(None).await?;
    let page = engine.create_page(parent, context).await?;
    page.set_surface(
        nomifun_browser_platform::runtime::BrowserSurfaceBounds {
            x: 20., y: 60., width: 1060., height: 620.,
        },
        true,
        Default::default(),
    ).await?;
    let view = native::View::new(page.clone());
    let automation = Arc::new(tokio::sync::Mutex::new(automation::TabAutomation::default()));
    let locked = Arc::new(AtomicBool::new(false));
    let chooser = macos::user_file_chooser::UserFileChooser::install(
        app.clone(), view, automation, locked, initial_directory.to_path_buf(),
    ).await?;
    page.set_input_locked(false).await?;
    chooser.set_visible(true);
    navigate_fixture(&page, url).await?;
    for _ in 0..100 {
        if evaluate(&page, "window.userFileFixtureReady===true").await? == true { break; }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let file = point(&page, "file").await?;
    queue_user_click(app, file).await?;
    chooser.wait_for_panel().await?;
    chooser.set_visible(false);
    chooser.wait_for_idle().await?;
    checks["native_user_file_picker_cancel_on_hide"] =
        (evaluate(&page, "userFiles.length").await? == 0).into();
    chooser.close().await?;
    page.force_close().await?;
    eprintln!("CEF_SMOKE_PHASE user_file_picker_cancel_settled");
    Ok(())
}

#[cfg(target_os = "macos")]
async fn verify_user_downloads(
    engine: &std::sync::Arc<nomifun_browser_macos::engine::Engine>,
    app: &tauri::AppHandle,
    parent: std::sync::Arc<nomifun_browser_macos::engine::ParentView>,
    url: &str,
    checks: &mut serde_json::Value,
) -> Result<(), String> {
    use nomifun_browser_platform::runtime::{BrowserDownloadState, BrowserSurfaceBounds};
    let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
    let context = engine.create_context(None).await?;
    let page = engine.create_page(parent, context).await?;
    page.configure_user_downloads(directory.path().to_path_buf())?;
    page.set_surface(
        BrowserSurfaceBounds { x: 20., y: 60., width: 1060., height: 620. },
        true,
        Default::default(),
    ).await?;
    page.set_input_locked(false).await?;
    navigate_fixture(&page, url).await?;

    let mut changes = page.subscribe();
    queue_user_click(app, point(&page, "download").await?).await?;
    let completed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(download) = page.user_download_snapshot().into_iter()
                .find(|download| download.state == BrowserDownloadState::Completed)
            {
                return Ok::<_, String>(download);
            }
            changes.changed().await.map_err(|_| "CEF user download state closed".to_owned())?;
        }
    }).await.map_err(|_| "CEF user download did not complete".to_owned())??;
    let completed_bytes = std::fs::read(directory.path().join(&completed.filename))
        .map_err(|error| error.to_string())?;
    checks["native_user_download"] =
        (completed_bytes == "Native CEF download 中文\n".as_bytes()).into();

    queue_user_click(app, point(&page, "cancel").await?).await?;
    let cancellable = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let Some(download) = page.user_download_snapshot().into_iter()
                .find(|download| download.state == BrowserDownloadState::InProgress && download.can_cancel)
            {
                return Ok::<_, String>(download);
            }
            changes.changed().await.map_err(|_| "CEF cancellable download state closed".to_owned())?;
        }
    }).await.map_err(|_| "CEF user download was never cancellable".to_owned())??;
    page.cancel_user_download(&cancellable.id)?;
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if page.user_download_snapshot().iter().any(|download|
                download.id == cancellable.id && download.state == BrowserDownloadState::Cancelled)
            {
                return Ok::<_, String>(());
            }
            changes.changed().await.map_err(|_| "CEF cancelled download state closed".to_owned())?;
        }
    }).await.map_err(|_| "CEF user download cancellation did not settle".to_owned())??;
    let files = std::fs::read_dir(directory.path())
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .count();
    checks["native_user_download_cancel"] = (files == 1).into();
    page.force_close().await?;
    eprintln!("CEF_SMOKE_PHASE user_downloads_settled");
    Ok(())
}

#[cfg(target_os = "macos")]
async fn verify_permissions(
    engine: &std::sync::Arc<nomifun_browser_macos::engine::Engine>,
    app: &tauri::AppHandle,
    parent: std::sync::Arc<nomifun_browser_macos::engine::ParentView>,
    url: &str,
    checks: &mut serde_json::Value,
) -> Result<(), String> {
    let context = engine.create_context(None).await?;
    let page = engine.create_page(parent, context).await?;
    page.set_surface(
        nomifun_browser_platform::runtime::BrowserSurfaceBounds {
            x: 20., y: 60., width: 1060., height: 620.,
        },
        true,
        Default::default(),
    ).await?;
    page.protocol.call(None, "Page.enable", serde_json::json!({})).await?;
    navigate_fixture(&page, url).await?;
    page.set_input_locked(false).await?;
    let geo = point(&page, "geo").await?;
    queue_user_click(app, geo).await?;
    let mut changes = page.subscribe();
    let request = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let snapshot = changes.borrow_and_update().clone();
            if let Some(request) = snapshot.permission_requests.first().cloned() {
                return Ok::<_, String>((snapshot.document_generation, request));
            }
            changes.changed().await.map_err(|_| "CEF permission state closed".to_owned())?;
        }
    }).await.map_err(|_| "CEF user permission request was not projected".to_owned())??;
    page.reply_permission(request.1.request_id, request.0, false).await?;
    for _ in 0..100 {
        if evaluate(&page, "String(window.geoResult||'')").await?.as_str().is_some_and(|value| value.starts_with("denied-")) { break; }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    checks["native_user_permission_denial"] =
        (evaluate(&page, "String(window.geoResult||'')").await?.as_str().is_some_and(|value| value.starts_with("denied-"))
            && page.snapshot().permission_requests.is_empty()).into();

    page.set_input_locked(true).await?;
    let denied = async_evaluate(
        &page,
        "new Promise(resolve=>navigator.geolocation.getCurrentPosition(()=>resolve('allowed'),()=>resolve('denied')))",
    ).await?;
    checks["native_agent_permission_fail_closed"] =
        (denied == "denied" && page.snapshot().permission_requests.is_empty()
            && page.snapshot().blocked_permissions.iter().any(|kind|kind=="geolocation")).into();
    page.force_close().await?;
    eprintln!("CEF_SMOKE_PHASE permissions_settled");
    Ok(())
}

#[cfg(target_os = "macos")]
async fn navigate_fixture(page: &nomifun_browser_macos::engine::Page, url: &str) -> Result<(), String> {
    let previous = page.snapshot().document_generation;
    page.protocol.call(None, "Page.enable", serde_json::json!({})).await?;
    page.protocol.call(None, "Page.navigate", serde_json::json!({"url":url})).await?;
    let mut changes = page.subscribe();
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let state = changes.borrow_and_update().clone();
            if state.document_generation > previous && state.lifecycle == nomifun_browser_platform::runtime::BrowserTabLifecycle::Ready { return Ok(()); }
            changes.changed().await.map_err(|_| "Storage fixture navigation closed".to_owned())?;
        }
    }).await.map_err(|_| "Storage fixture navigation timed out".to_owned())?
}

#[cfg(target_os = "macos")]
async fn verify_renderer_crash(
    engine: &std::sync::Arc<nomifun_browser_macos::engine::Engine>,
    parent: std::sync::Arc<nomifun_browser_macos::engine::ParentView>,
    url: &str,
    checks: &mut serde_json::Value,
) -> Result<(), String> {
    use nomifun_browser_platform::runtime::{BrowserSurfaceBounds, BrowserTabLifecycle};

    let context = engine.create_context(None).await?;
    let page = engine.create_page(parent, context).await?;
    page.set_surface(
        BrowserSurfaceBounds {
            x: 20.,
            y: 60.,
            width: 1060.,
            height: 620.,
        },
        true,
        Default::default(),
    )
    .await?;
    navigate_fixture(&page, url).await?;
    let mut changes = page.subscribe();
    let protocol = page.protocol.clone();
    let crash_call = tokio::spawn(async move {
        protocol.call(None, "Page.crash", serde_json::json!({})).await
    });
    let crashed = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let state = changes.borrow_and_update().clone();
            if state.lifecycle == BrowserTabLifecycle::Crashed {
                return Ok::<_, String>(state);
            }
            changes
                .changed()
                .await
                .map_err(|_| "CEF crash state subscription closed".to_owned())?;
        }
    })
    .await
    .map_err(|_| "CEF renderer crash was not projected".to_owned())??;
    let command_failed_closed = tokio::time::timeout(std::time::Duration::from_secs(2), crash_call)
        .await
        .map_err(|_| "CEF crash command did not settle".to_owned())?
        .map_err(|_| "CEF crash command task failed".to_owned())?
        .is_err();
    checks["native_renderer_crash_projection"] = (crashed.lifecycle == BrowserTabLifecycle::Crashed
        && page.input_locked()
        && page.protocol.is_closed()
        && command_failed_closed)
        .into();
    page.force_close().await?;
    eprintln!("CEF_SMOKE_PHASE renderer_crash_settled");
    Ok(())
}

#[cfg(target_os = "macos")]
async fn navigate_storage_fixture(
    page: &std::sync::Arc<nomifun_browser_macos::engine::Page>,
    url: &str,
) -> Result<(), String> {
    use nomifun_browser_platform::runtime::BrowserSurfaceBounds;
    page.set_surface(
        BrowserSurfaceBounds {
            x: 20.,
            y: 60.,
            width: 1060.,
            height: 620.,
        },
        true,
        Default::default(),
    )
    .await?;
    let navigation = navigate_fixture(page, url).await;
    let hidden = page.hide().await;
    navigation?;
    hidden
}
#[cfg(target_os = "macos")]
async fn async_evaluate(page: &nomifun_browser_macos::engine::Page, expression: &str) -> Result<serde_json::Value, String> {
    let value = page.protocol.call(None, "Runtime.evaluate", serde_json::json!({"expression":expression,"returnByValue":true,"awaitPromise":true})).await?;
    if value.get("exceptionDetails").is_some() { return Err("Storage fixture script failed".into()); }
    Ok(value["result"]["value"].clone())
}
#[cfg(target_os = "macos")]
async fn verify_storage(engine: &std::sync::Arc<nomifun_browser_macos::engine::Engine>, parent: std::sync::Arc<nomifun_browser_macos::engine::ParentView>, root: &std::path::Path, url: &str, checks: &mut serde_json::Value) -> Result<(), String> {
    const WRITE: &str = "(async()=>{localStorage.setItem('nomi','owned');document.cookie='nomi=owned;max-age=3600;path=/';const db=await new Promise((resolve,reject)=>{const r=indexedDB.open('nomi-fixture',1);r.onupgradeneeded=()=>r.result.createObjectStore('items');r.onsuccess=()=>resolve(r.result);r.onerror=()=>reject(r.error)});await new Promise((resolve,reject)=>{const t=db.transaction('items','readwrite');t.objectStore('items').put('owned','key');t.oncomplete=resolve;t.onerror=()=>reject(t.error)});db.close();const cache=await caches.open('nomi-fixture');await cache.put('/cached',new Response('owned'));return true})()";
    const READ: &str = "(async()=>({local:localStorage.getItem('nomi'),cookie:document.cookie,databases:(await indexedDB.databases()).map(d=>d.name),caches:await caches.keys()}))()";
    let a = engine.create_context(Some(root.join("conversation-a"))).await?;
    let b = engine.create_context(Some(root.join("conversation-b"))).await?;
    let page_a = engine.create_page(parent.clone(), a.clone()).await?;
    navigate_storage_fixture(&page_a, url)
        .await
        .map_err(|error| format!("storage context A navigation failed: {error}"))?;
    // Do not overlap bootstrap navigation for two fresh persistent request
    // contexts. CEF can allocate both renderer hosts before either DevTools
    // observer has received its first navigation reply; serial creation keeps
    // the same two-context isolation contract with deterministic ownership.
    let page_b = engine.create_page(parent.clone(), b.clone()).await?;
    navigate_storage_fixture(&page_b, url)
        .await
        .map_err(|error| format!("storage context B navigation failed: {error}"))?;
    eprintln!("CEF_SMOKE_PHASE storage_contexts_navigated");
    async_evaluate(&page_a, WRITE).await?;
    let isolated = async_evaluate(&page_b, READ).await?;
    checks["conversation_storage_isolation"] = (isolated["local"].is_null() && isolated["cookie"] == "" && isolated["databases"] == serde_json::json!([]) && isolated["caches"] == serde_json::json!([])).into();
    async_evaluate(&page_b, WRITE).await?;
    eprintln!("CEF_SMOKE_PHASE storage_contexts_written");
    page_a.force_close().await?;
    let recreated = engine.create_page(parent.clone(), a.clone()).await?;
    navigate_storage_fixture(&recreated, url).await?;
    let persisted = async_evaluate(&recreated, READ).await?;
    checks["conversation_survives_tab_recreation"] = (persisted["local"] == "owned" && persisted["cookie"] == "nomi=owned" && persisted["databases"] == serde_json::json!(["nomi-fixture"]) && persisted["caches"] == serde_json::json!(["nomi-fixture"])).into();
    eprintln!("CEF_SMOKE_PHASE storage_tab_recreated");
    // Same context at a second origin, to prove that clearing is not restricted
    // to the most recently visible site's origin.
    let second = url.replace("127.0.0.1", "localhost");
    navigate_storage_fixture(&recreated, &second).await?;
    async_evaluate(&recreated, WRITE).await?;
    let maintenance = engine.create_page(parent.clone(), a.clone()).await?;
    maintenance.protocol.call(None, "Page.enable", serde_json::json!({})).await?;
    checks["clear_rejects_live_sibling"] = maintenance.clear_site_data(Default::default()).await.is_err().into();
    eprintln!("CEF_SMOKE_PHASE storage_live_sibling_rejected");
    recreated.force_close().await?;
    let cancelled = tokio_util::sync::CancellationToken::new(); cancelled.cancel();
    checks["clear_rejects_cancelled_request"] = maintenance.clear_site_data(cancelled).await.is_err().into();
    eprintln!("CEF_SMOKE_PHASE storage_cancel_rejected");
    maintenance.clear_site_data(Default::default()).await?;
    eprintln!("CEF_SMOKE_PHASE storage_clear_completed");
    maintenance.force_close().await?;
    let cleared = engine.create_page(parent.clone(), a).await?;
    let mut all_cleared = true;
    for origin in [url, &second] {
        navigate_storage_fixture(&cleared, origin).await?;
        let value = async_evaluate(&cleared, READ).await?;
        all_cleared &= value["local"].is_null() && value["cookie"] == "" && value["databases"] == serde_json::json!([]) && value["caches"] == serde_json::json!([]);
    }
    checks["clear_all_conversation_origins"] = all_cleared.into();
    checks["clear_preserves_other_conversation"] = (async_evaluate(&page_b, READ).await? == persisted).into();
    cleared.force_close().await?;
    page_b.force_close().await?;
    eprintln!("CEF_SMOKE_PHASE context_isolation_and_clear_settled");
    Ok(())
}

#[cfg(target_os = "macos")]
async fn verify_runtime(engine: &std::sync::Arc<nomifun_browser_macos::engine::Engine>, app: &tauri::AppHandle, url: &str, checks: &mut serde_json::Value) -> Result<(), String> {
    use nomifun_browser_platform::{runtime::*, run_guard::{BrowserRunCoordinator, BrowserInputState}};
    let host = macos::host::DesktopBrowserHost::new(app.clone(), engine.clone());
    let runtime = host.create(CreateBrowserRuntime { key: BrowserResourceKey { principal_id:"fixture".into(), agent_session_id:"native-cef".into(), resource_binding_id:"browser-fixture:native-cef".into() }, runtime_generation: 7, profile: BrowserProfile::Ephemeral, user_input_enabled:true }).await.map_err(|e|e.to_string())?;
    runtime.surface().unwrap().set_surface(BrowserSurfaceBounds { x:20., y:60., width:1060., height:620. }, true, Default::default()).await.map_err(|e|e.to_string())?;
    runtime.execute(BrowserTabCommand::Create { url:url.into() }, Default::default()).await.map_err(|e|e.to_string())?;
    let mut changes = runtime.changes().unwrap();
    let target = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let snapshot = runtime.snapshot().await.map_err(|e|e.to_string())?;
            if let Some(tab) = snapshot.tabs.iter().find(|tab|tab.url.starts_with(url) && tab.lifecycle == BrowserTabLifecycle::Ready) { return Ok::<_,String>(tab.target.clone()); }
            changes.changed().await.map_err(|_| "Runtime metadata subscription closed")?;
        }
    }).await.map_err(|_| "Runtime navigation metadata timed out")??;
    let coordinator = BrowserRunCoordinator::new(runtime.clone());
    let run = coordinator.begin().await.map_err(|e|e.to_string())?;
    run.require_explicit_finish();
    let observation = { let runtime = runtime.clone(); coordinator.agent_operation(&run, move |cancel| async move { Ok(runtime.automation().unwrap().observe(None, cancel).await) }).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())? };
    checks["runtime_semantic_observation"] = (observation.target == target && observation.elements.iter().any(|e|e.name == "输入内容")).into();
    let screenshot = { let runtime = runtime.clone(); coordinator.agent_operation(&run, move |cancel| async move { Ok(runtime.automation().unwrap().screenshot(None, cancel).await) }).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())? };
    checks["runtime_viewport_capture"] = (screenshot.width > 0 && screenshot.height > 0 && screenshot.width <= 1600 && screenshot.height <= 1600 && !screenshot.png_base64.is_empty()).into();
    let evaluated = { let runtime = runtime.clone(); let target = target.clone(); coordinator.agent_operation(&run, move |cancel| async move {
        Ok(runtime.automation().unwrap().evaluate(BrowserEvaluation { target, expression:"confirm('CEF RunGuard Stop')".into() }, cancel).await)
    }).await.map_err(|e|e.to_string())?.map_err(|e|e.to_string())? };
    checks["runtime_dialog_yields_owned_operation"] = matches!(evaluated.outcome, BrowserEvaluationOutcome::AwaitingDialog { .. }).into();
    run.cancel();
    coordinator.finish(&run).await.map_err(|e|e.to_string())?;
    let stopped = coordinator.snapshot().await;
    checks["runtime_stop_settles_before_unlock"] = (stopped.input_state == BrowserInputState::UserReady && !stopped.input_gate_failed && runtime.snapshot().await.map_err(|e|e.to_string())?.tabs.iter().all(|tab|tab.script_dialog.is_none())).into();

    let popup_url = format!("{url}/popup-source");
    runtime.execute(BrowserTabCommand::Navigate { target: target.clone(), url: popup_url.clone() }, Default::default()).await.map_err(|error|error.to_string())?;
    let popup_source = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let snapshot = runtime.snapshot().await.map_err(|error|error.to_string())?;
            if let Some(tab) = snapshot.tabs.iter().find(|tab|tab.url.ends_with("/popup-source") && tab.lifecycle==BrowserTabLifecycle::Ready) {
                return Ok::<_,String>(tab.target.clone());
            }
            changes.changed().await.map_err(|_|"Runtime popup source subscription closed".to_owned())?;
        }
    }).await.map_err(|_|"Runtime popup source timed out".to_owned())??;
    let popup_run = coordinator.begin().await.map_err(|error|error.to_string())?;
    popup_run.require_explicit_finish();
    let observed = { let runtime=runtime.clone(); coordinator.agent_operation(&popup_run,move |cancel|async move {Ok(runtime.automation().unwrap().observe(None,cancel).await)}).await.map_err(|error|error.to_string())?.map_err(|error|error.to_string())? };
    let open = observed.elements.iter().find(|element|element.name=="Open real popup").ok_or("Native CEF popup trigger is missing")?.reference.clone();
    { let runtime=runtime.clone(); coordinator.agent_operation(&popup_run,move |cancel|async move {Ok(runtime.automation().unwrap().act(BrowserAction::click(open),cancel).await)}).await.map_err(|error|error.to_string())?.map_err(|error|error.to_string())?; }
    let popup = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let snapshot=runtime.snapshot().await.map_err(|error|error.to_string())?;
            if let Some(tab)=snapshot.tabs.iter().find(|tab|tab.target.tab_id!=popup_source.tab_id && tab.url.ends_with("/popup-child") && tab.lifecycle==BrowserTabLifecycle::Ready) {
                return Ok::<_,String>(tab.target.clone());
            }
            changes.changed().await.map_err(|_|"Runtime popup subscription closed".to_owned())?;
        }
    }).await.map_err(|_|"Native CEF popup admission timed out".to_owned())??;
    let proof = { let runtime=runtime.clone(); let popup=popup.clone(); coordinator.agent_operation(&popup_run,move |cancel|async move {Ok(runtime.automation().unwrap().evaluate(BrowserEvaluation {target:popup,expression:"({hasOpener:!!opener,cookie:document.cookie,ownTop:top===window})".into()},cancel).await)}).await.map_err(|error|error.to_string())?.map_err(|error|error.to_string())? };
    checks["runtime_native_popup_opener_and_profile"] = matches!(proof.outcome,BrowserEvaluationOutcome::Completed { value } if value["hasOpener"]==true && value["ownTop"]==true && value["cookie"].as_str().is_some_and(|cookie|cookie.contains("nomi_popup=shared-profile"))).into();
    coordinator.finish(&popup_run).await.map_err(|error|error.to_string())?;
    eprintln!("CEF_SMOKE_PHASE runtime_popup_settled");

    runtime.execute(BrowserTabCommand::Activate { target: popup_source.clone() }, Default::default()).await.map_err(|error|error.to_string())?;
    runtime.execute(BrowserTabCommand::Navigate { target: popup_source.clone(), url: format!("{url}/download-source") }, Default::default()).await.map_err(|error|error.to_string())?;
    let download_source = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let snapshot=runtime.snapshot().await.map_err(|error|error.to_string())?;
            if let Some(tab)=snapshot.tabs.iter().find(|tab|tab.target.tab_id==popup_source.tab_id && tab.url.ends_with("/download-source") && tab.lifecycle==BrowserTabLifecycle::Ready) {
                return Ok::<_,String>(tab.target.clone());
            }
            changes.changed().await.map_err(|_|"Runtime download source subscription closed".to_owned())?;
        }
    }).await.map_err(|_|"Runtime download source timed out".to_owned())??;
    let download_root=tempfile::tempdir().map_err(|error|error.to_string())?;
    let download_scope=std::sync::Arc::new(nomifun_browser_platform::downloads::BrowserDownloadScope::open(download_root.path()).map_err(|error|error.to_string())?);
    let prepared=download_scope.prepare().map_err(|error|error.to_string())?;
    let download_run=coordinator.begin().await.map_err(|error|error.to_string())?;
    download_run.require_explicit_finish();
    let observed={let runtime=runtime.clone();let id=download_source.tab_id.clone();coordinator.agent_operation(&download_run,move |cancel|async move {Ok(runtime.automation().unwrap().observe(Some(id),cancel).await)}).await.map_err(|error|error.to_string())?.map_err(|error|error.to_string())?};
    let link=observed.elements.iter().find(|element|element.name=="Download local fixture").ok_or("Native CEF download trigger is missing")?.reference.clone();
    let downloaded={let runtime=runtime.clone();let prepared=prepared.clone();coordinator.agent_operation(&download_run,move |cancel|async move {Ok(runtime.automation().unwrap().download(link,prepared,cancel).await)}).await.map_err(|error|error.to_string())?.map_err(|error|error.to_string())?};
    let artifact=downloaded.download.ok_or("Native CEF download artifact is missing")?;
    let bytes=std::fs::read(download_root.path().join(&artifact.path)).map_err(|error|error.to_string())?;
    checks["runtime_native_download_publication"]=(bytes=="Native CEF download 中文\n".as_bytes() && artifact.bytes==bytes.len() as u64).into();
    coordinator.finish(&download_run).await.map_err(|error|error.to_string())?;
    eprintln!("CEF_SMOKE_PHASE runtime_download_settled");
    runtime.close().await.map_err(|e|e.to_string())?;
    eprintln!("CEF_SMOKE_PHASE runtime_guard_stop_settled");
    Ok(())
}

#[cfg(target_os = "macos")]
async fn verify_runtime_soak(
    engine: &std::sync::Arc<nomifun_browser_macos::engine::Engine>,
    app: &tauri::AppHandle,
    base_url: &str,
) -> Result<serde_json::Value, String> {
    use nomifun_browser_platform::{
        run_guard::{BrowserInputState, BrowserRunCoordinator},
        runtime::{
            BrowserAction, BrowserEvaluation, BrowserEvaluationOutcome, BrowserProfile,
            BrowserResourceKey, BrowserRuntimeFactory, BrowserTabCommand, BrowserTabLifecycle,
            CreateBrowserRuntime, WorkspaceError,
        },
    };
    use std::time::Instant;

    fn percentile(values: &[u128], numerator: usize, denominator: usize) -> u128 {
        let mut sorted = values.to_vec();
        sorted.sort_unstable();
        let index = ((sorted.len() - 1) * numerator).div_ceil(denominator);
        sorted[index.min(sorted.len() - 1)]
    }

    let host = macos::host::DesktopBrowserHost::new(app.clone(), engine.clone());
    let runtime = host.create(CreateBrowserRuntime {
        key: BrowserResourceKey {
            principal_id: "fixture".into(),
            agent_session_id: "native-cef-soak".into(),
            resource_binding_id: "browser-fixture:native-cef-soak".into(),
        },
        runtime_generation: 18,
        profile: BrowserProfile::Ephemeral,
        user_input_enabled: true,
    }).await.map_err(|error| error.to_string())?;
    runtime.surface().ok_or("Native CEF soak surface is missing")?
        .set_surface(
            nomifun_browser_platform::runtime::BrowserSurfaceBounds {
                x: 20., y: 60., width: 1060., height: 620.,
            },
            true,
            Default::default(),
        ).await.map_err(|error| error.to_string())?;

    let initial_url = format!("{base_url}/browser_workspace.html?cycle=initial");
    runtime.execute(
        BrowserTabCommand::Create { url: initial_url.clone() },
        Default::default(),
    ).await.map_err(|error| error.to_string())?;
    let mut changes = runtime.changes().ok_or("Native CEF soak changes are unavailable")?;
    let mut target = tokio::time::timeout(std::time::Duration::from_secs(8), async {
        loop {
            let snapshot = runtime.snapshot().await.map_err(|error| error.to_string())?;
            if let Some(tab) = snapshot.tabs.iter().find(|tab|
                tab.url == initial_url && tab.lifecycle == BrowserTabLifecycle::Ready
            ) {
                return Ok::<_, String>(tab.target.clone());
            }
            changes.changed().await.map_err(|_| "Native CEF soak subscription closed".to_owned())?;
        }
    }).await.map_err(|_| "Native CEF soak initial navigation timed out".to_owned())??;
    let stable_tab_id = target.tab_id.clone();
    let coordinator = BrowserRunCoordinator::new(runtime.clone());
    let run = coordinator.begin().await.map_err(|error| error.to_string())?;
    run.require_explicit_finish();

    let cycle_result = async {
        let mut durations_ms = Vec::with_capacity(100);
        let mut stale_reference = None;
        let mut stale_target_rejections = 0usize;
        let mut generations = Vec::with_capacity(100);
        for cycle in 0..100usize {
            let started = Instant::now();
            let url = format!("{base_url}/browser_workspace.html?cycle={cycle:03}");
            let previous_generation = target.document_generation;
            runtime.execute(
                BrowserTabCommand::Navigate { target: target.clone(), url: url.clone() },
                Default::default(),
            ).await.map_err(|error| error.to_string())?;
            target = tokio::time::timeout(std::time::Duration::from_secs(8), async {
                loop {
                    let snapshot = runtime.snapshot().await.map_err(|error| error.to_string())?;
                    if let Some(tab) = snapshot.tabs.iter().find(|tab|
                        tab.target.tab_id == stable_tab_id
                            && tab.url == url
                            && tab.lifecycle == BrowserTabLifecycle::Ready
                            && tab.target.document_generation > previous_generation
                    ) {
                        return Ok::<_, String>(tab.target.clone());
                    }
                    changes.changed().await.map_err(|_| "Native CEF soak subscription closed".to_owned())?;
                }
            }).await.map_err(|_| format!("Native CEF soak navigation {cycle:03} timed out"))??;
            generations.push(target.document_generation);

            let mut fixture_ready = false;
            for _ in 0..100 {
                let readiness = {
                    let runtime = runtime.clone();
                    let target = target.clone();
                    coordinator.agent_operation(&run, move |cancel| async move {
                        Ok(runtime.automation().expect("CEF automation").evaluate(
                            BrowserEvaluation {
                                target,
                                expression: "document.readyState==='complete'&&document.querySelector('#field')!==null&&document.querySelector('#result')?.textContent==='等待 Agent'".into(),
                            },
                            cancel,
                        ).await)
                    }).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?
                };
                if matches!(readiness.outcome, BrowserEvaluationOutcome::Completed { value } if value == true) {
                    fixture_ready = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            if !fixture_ready {
                return Err(format!("Cycle {cycle:03} reached Ready before the fixture script settled"));
            }

            if let Some(reference) = stale_reference.take() {
                let stale = {
                    let runtime = runtime.clone();
                    coordinator.agent_operation(&run, move |cancel| async move {
                        Ok(runtime.automation().expect("CEF automation").act(
                            BrowserAction::click(reference), cancel,
                        ).await)
                    }).await.map_err(|error| error.to_string())?
                };
                if !matches!(stale, Err(WorkspaceError::StaleTarget)) {
                    return Err(format!("Cycle {cycle:03} did not reject the previous-document target exactly: {stale:?}"));
                }
                stale_target_rejections += 1;
            }

            let observation = {
                let runtime = runtime.clone();
                let tab_id = stable_tab_id.clone();
                coordinator.agent_operation(&run, move |cancel| async move {
                    Ok(runtime.automation().expect("CEF automation").observe(Some(tab_id), cancel).await)
                }).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?
            };
            if observation.target != target {
                return Err(format!("Cycle {cycle:03} observation target drifted"));
            }
            let field = observation.elements.iter()
                .find(|element| element.name == "输入内容" && element.role == "textbox")
                .ok_or_else(|| format!("Cycle {cycle:03} omitted the textbox"))?
                .reference.clone();
            let text = format!("soak-{cycle:03}-中文");
            {
                let runtime = runtime.clone();
                let text = text.clone();
                coordinator.agent_operation(&run, move |cancel| async move {
                    Ok(runtime.automation().expect("CEF automation").act(
                        BrowserAction::Type { element: field, text }, cancel,
                    ).await)
                }).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
            }
            let observation = {
                let runtime = runtime.clone();
                let tab_id = stable_tab_id.clone();
                coordinator.agent_operation(&run, move |cancel| async move {
                    Ok(runtime.automation().expect("CEF automation").observe(Some(tab_id), cancel).await)
                }).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?
            };
            let button = observation.elements.iter()
                .find(|element| element.name == "验证点击" && element.role == "button")
                .ok_or_else(|| format!("Cycle {cycle:03} omitted the click target"))?
                .reference.clone();
            {
                let runtime = runtime.clone();
                coordinator.agent_operation(&run, move |cancel| async move {
                    Ok(runtime.automation().expect("CEF automation").act(
                        BrowserAction::click(button), cancel,
                    ).await)
                }).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
            }
            let observation = {
                let runtime = runtime.clone();
                let tab_id = stable_tab_id.clone();
                coordinator.agent_operation(&run, move |cancel| async move {
                    Ok(runtime.automation().expect("CEF automation").observe(Some(tab_id), cancel).await)
                }).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?
            };
            stale_reference = Some(observation.elements.iter()
                .find(|element| element.name == "验证点击" && element.role == "button")
                .ok_or_else(|| format!("Cycle {cycle:03} omitted the final click target"))?
                .reference.clone());

            let state = runtime.snapshot().await.map_err(|error| error.to_string())?;
            let tab = state.tabs.iter().find(|tab| tab.target.tab_id == stable_tab_id)
                .ok_or_else(|| format!("Cycle {cycle:03} lost the stable tab"))?;
            if state.tabs.len() != 1
                || state.active_tab_id.as_deref() != Some(stable_tab_id.as_str())
                || !state.downloads.is_empty()
                || !tab.blocked_permissions.is_empty()
                || !tab.permission_requests.is_empty()
                || tab.script_dialog.is_some()
            {
                return Err(format!("Cycle {cycle:03} leaked native Browser state"));
            }
            let evaluated = {
                let runtime = runtime.clone();
                let target = target.clone();
                coordinator.agent_operation(&run, move |cancel| async move {
                    Ok(runtime.automation().expect("CEF automation").evaluate(
                        BrowserEvaluation {
                            target,
                            expression: "(()=>{const field=document.querySelector('#field');const result=document.querySelector('#result');return {value:field?.value,clicks:Number(result?.dataset.clicks||0),trusted:result?.dataset.lastClickTrusted==='true',result:result?.textContent}})()".into(),
                        },
                        cancel,
                    ).await)
                }).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?
            };
            let result = match evaluated.outcome {
                BrowserEvaluationOutcome::Completed { value } => value,
                outcome => return Err(format!("Cycle {cycle:03} evaluation did not complete: {outcome:?}")),
            };
            if result["value"] != text || result["clicks"] != 1 || result["trusted"] != true {
                return Err(format!("Cycle {cycle:03} native result mismatch: {result}"));
            }
            durations_ms.push(started.elapsed().as_millis());
            if cycle % 10 == 9 {
                eprintln!("CEF_SOAK_PHASE cycles_completed={}", cycle + 1);
            }
        }
        Ok::<_, String>((durations_ms, generations, stale_target_rejections))
    }.await;

    if cycle_result.is_err() {
        run.cancel();
    }
    let finish = coordinator.finish(&run).await.map_err(|error| error.to_string());
    let gate = coordinator.snapshot().await;
    let close = runtime.close().await.map_err(|error| error.to_string());
    let (durations_ms, generations, stale_target_rejections) = match (cycle_result, finish, close) {
        (Ok(metrics), Ok(()), Ok(())) => metrics,
        (cycles, finish, close) => {
            return Err(format!("Native CEF soak cycles={cycles:?}; finish={finish:?}; close={close:?}"));
        }
    };
    if gate.input_state != BrowserInputState::UserReady || gate.input_gate_failed {
        return Err("Native CEF soak did not settle its input gate".into());
    }
    if generations.windows(2).any(|pair| pair[1] <= pair[0]) {
        return Err("Native CEF soak document generations were not strictly increasing".into());
    }
    let first_median = percentile(&durations_ms[..20], 1, 2);
    let last_median = percentile(&durations_ms[80..], 1, 2);
    let first_p95 = percentile(&durations_ms[..20], 95, 100);
    let last_p95 = percentile(&durations_ms[80..], 95, 100);
    let latency_stable = last_median <= first_median.saturating_mul(3).saturating_add(250)
        && last_p95 <= first_p95.saturating_mul(4).saturating_add(500);
    let checks = serde_json::json!({
        "cycles_completed": durations_ms.len() == 100,
        "strict_document_generations": generations.len() == 100,
        "previous_document_refs_rejected": stale_target_rejections == 99,
        "single_runtime_and_tab": true,
        "unicode_type_and_trusted_click": true,
        "zero_cycle_errors": true,
        "input_gate_settled": true,
        "latency_not_sequence_degraded": latency_stable,
    });
    Ok(serde_json::json!({
        "scope": "native-cef-100-cycle-soak",
        "checks": checks,
        "metrics": {
            "cycles": durations_ms.len(),
            "stale_target_rejections": stale_target_rejections,
            "first_20_median_ms": first_median,
            "last_20_median_ms": last_median,
            "first_20_p95_ms": first_p95,
            "last_20_p95_ms": last_p95,
            "max_cycle_ms": durations_ms.iter().copied().max().unwrap_or(0),
            "first_document_generation": generations.first(),
            "last_document_generation": generations.last(),
        },
        "passed": checks.as_object().is_some_and(|values| values.values().all(|value| value == true)),
    }))
}

#[cfg(target_os = "macos")]
async fn set_fixture_browser_zoom(view: &native::View, factor: f64) -> Result<(), String> {
    view.page.set_zoom_factor(factor).await
}
