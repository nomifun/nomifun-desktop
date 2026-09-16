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
    std::fs::write(report.with_extension("pid"), std::process::id().to_string()).expect("fixture PID receipt");
    let root = tempfile::Builder::new().prefix("nomi-cef-smoke-").tempdir().expect("disposable CEF profile");
    let data = root.path().to_path_buf();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break; };
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut request = [0u8; 4096];
                let Ok(count) = stream.read(&mut request) else { return; };
                let request = String::from_utf8_lossy(&request[..count]);
                let path = request.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/");
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
                } else { include_str!("fixtures/browser_workspace.html")
                    .replace("<div id=\"scroller\">", "<div id=\"scroller\" role=\"region\" aria-label=\"滚动区域\">")
                    .replace("<div id=\"drag\">", "<div id=\"drag\" role=\"button\" aria-label=\"拖拽区域\">") };
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            });
        }
    });
    let bundle = std::env::current_exe().unwrap().parent().unwrap().parent().unwrap().parent().unwrap().to_path_buf();
    let helper = bundle.join("Contents/Frameworks/NomiCEFSmoke Helper.app/Contents/MacOS/NomiCEFSmoke Helper");
    let paths = Paths { framework: bundle.join("Contents/Frameworks/Chromium Embedded Framework.framework"), helper, main_bundle: bundle, data_root: data.clone() };
    let engine_slot = Arc::new(std::sync::OnceLock::<Arc<Engine>>::new());
    let setup_engine = engine_slot.clone();
    let failure_report = report.clone();
    let app = tauri::Builder::default().setup(move |app| {
        tauri::window::WindowBuilder::new(app, "main").title("NomiFun — macOS CEF native conformance")
            .inner_size(1100.0, 720.0).min_inner_size(880.0, 600.0).build()?;
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
            tokio::time::sleep(std::time::Duration::from_secs(90)).await;
            eprintln!("CEF_SMOKE_FAIL native operation or shutdown timed out");
            watchdog.exit(2);
        });
        tauri::async_runtime::spawn(async move {
            let result = async {
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
                let frames = engine.create_page(parent.clone(), context.clone()).await?;
                frames.set_surface(BrowserSurfaceBounds { x:20., y:60., width:1060., height:620. }, true, Default::default()).await?;
                browser_frame_input::verify_frame_input(&native::View::new(frames.clone()), &format!("http://{address}/frame-sessions")).await?;
                checks["native_frame_input_and_geometry"] = true.into();
                frames.force_close().await?;
                let uploads = engine.create_page(parent.clone(), context).await?;
                uploads.set_surface(BrowserSurfaceBounds { x:20., y:60., width:1060., height:620. }, true, Default::default()).await?;
                browser_upload_frames::verify(&native::View::new(uploads.clone()), &format!("http://{address}/upload-frames")).await?;
                checks["native_frame_uploads_and_stale_chooser"] = true.into();
                uploads.force_close().await?;
                verify_runtime(&engine, &handle, &format!("http://{address}"), &mut checks).await?;
                verify_storage(&engine, parent, &data, &format!("http://{address}"), &mut checks).await?;
                Ok::<_, String>(serde_json::json!({"checks":checks,"page":state,"passed":checks.as_object().unwrap().values().all(|v|v==true)}))
            }.await;
            let shutdown = engine.shutdown().await;
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
    let page_b = engine.create_page(parent.clone(), b.clone()).await?;
    navigate_fixture(&page_a, url).await?;
    navigate_fixture(&page_b, url).await?;
    async_evaluate(&page_a, WRITE).await?;
    let isolated = async_evaluate(&page_b, READ).await?;
    checks["conversation_storage_isolation"] = (isolated["local"].is_null() && isolated["cookie"] == "" && isolated["databases"] == serde_json::json!([]) && isolated["caches"] == serde_json::json!([])).into();
    async_evaluate(&page_b, WRITE).await?;
    page_a.force_close().await?;
    let recreated = engine.create_page(parent.clone(), a.clone()).await?;
    navigate_fixture(&recreated, url).await?;
    let persisted = async_evaluate(&recreated, READ).await?;
    checks["conversation_survives_tab_recreation"] = (persisted["local"] == "owned" && persisted["cookie"] == "nomi=owned" && persisted["databases"] == serde_json::json!(["nomi-fixture"]) && persisted["caches"] == serde_json::json!(["nomi-fixture"])).into();
    // Same context at a second origin, to prove that clearing is not restricted
    // to the most recently visible site's origin.
    let second = url.replace("127.0.0.1", "localhost");
    navigate_fixture(&recreated, &second).await?;
    async_evaluate(&recreated, WRITE).await?;
    let maintenance = engine.create_page(parent.clone(), a.clone()).await?;
    maintenance.protocol.call(None, "Page.enable", serde_json::json!({})).await?;
    checks["clear_rejects_live_sibling"] = maintenance.clear_site_data(Default::default()).await.is_err().into();
    recreated.force_close().await?;
    let cancelled = tokio_util::sync::CancellationToken::new(); cancelled.cancel();
    checks["clear_rejects_cancelled_request"] = maintenance.clear_site_data(cancelled).await.is_err().into();
    maintenance.clear_site_data(Default::default()).await?;
    maintenance.force_close().await?;
    let cleared = engine.create_page(parent.clone(), a).await?;
    let mut all_cleared = true;
    for origin in [url, &second] {
        navigate_fixture(&cleared, origin).await?;
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
    let runtime = host.create(CreateBrowserRuntime { key: BrowserWorkspaceKey { user_id:"fixture".into(), conversation_id:"native-cef".into() }, runtime_generation: 7, profile: BrowserProfile::Ephemeral, user_input_enabled:true }).await.map_err(|e|e.to_string())?;
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
    runtime.close().await.map_err(|e|e.to_string())?;
    eprintln!("CEF_SMOKE_PHASE runtime_guard_stop_settled");
    Ok(())
}

#[cfg(target_os = "macos")]
async fn set_fixture_browser_zoom(view: &native::View, factor: f64) -> Result<(), String> {
    view.page.set_zoom_factor(factor).await
}
