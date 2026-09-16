//! Real native presentation notifications across human and Agent lifetimes.
use nomifun_browser_platform::{runtime::*, workspace::BrowserWorkspaceService};
use std::sync::{Arc, Mutex};
use tauri::{Listener, Manager};
use tokio_util::sync::CancellationToken;

fn message(error: impl std::fmt::Display) -> String {
    error.to_string()
}

async fn native_visible(view: &tauri::Webview) -> Result<bool, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let mut visible = ::windows::core::BOOL::default();
        let result = unsafe { platform.controller().IsVisible(&mut visible) }
            .map(|_| visible.as_bool())
            .map_err(message);
        let _ = tx.send(result);
    })
    .map_err(message)?;
    rx.await.map_err(message)?
}

pub(super) async fn verify(app: &tauri::AppHandle, url: &str) -> Result<serde_json::Value, String> {
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let received = events.clone();
    let listener = app
        .get_window("main")
        .ok_or("Missing presentation window")?
        .listen("browser-workspace-open", move |event| {
            if let Ok(id) = serde_json::from_str::<String>(event.payload()) {
                received.lock().unwrap().push(id);
            }
        });
    let count = |expected: usize| -> Result<(), String> {
        let values = events.lock().unwrap();
        if values.len() != expected || values.iter().any(|id| id != "automatic-native") {
            return Err(format!(
                "Unexpected native presentation events: {values:?}, expected {expected}"
            ));
        }
        Ok(())
    };
    let service =
        BrowserWorkspaceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let key = BrowserWorkspaceKey {
        user_id: "presentation-fixture".into(),
        conversation_id: "automatic-native".into(),
    };
    let workspace = service
        .ensure_user(key.clone(), BrowserProfile::Ephemeral)
        .await
        .map_err(message)?;
    let result = async {
        let empty_run = workspace.begin_run().await.map_err(message)?;
        if !matches!(workspace.observe(&empty_run, None).await, Err(WorkspaceError::TabNotFound)) {
            return Err("Empty observe did not fail with TabNotFound".into());
        }
        count(0)?;
        workspace.finish_run(&empty_run).await.map_err(message)?;

        workspace.user_command(BrowserTabCommand::Create { url: url.into() }).await.map_err(message)?;
        let target = super::wait_workspace_page(&workspace, url).await?;
        let view = app.get_webview(&target.tab_id).ok_or("Missing native presentation fixture")?;
        let nonce = super::evaluate(&view, "popupNonce").await?;
        count(0)?; // User navigation is not an Agent auto-open request.

        let first = workspace.begin_run().await.map_err(message)?;
        workspace.agent_snapshot(&first).await.map_err(message)?;
        count(0)?; // Merely checking the inventory must not disturb the UI.
        workspace.observe(&first, Some(target.tab_id.clone())).await.map_err(message)?;
        count(1)?;
        workspace.observe(&first, Some(target.tab_id.clone())).await.map_err(message)?;
        count(1)?;
        workspace.agent_command(&first, BrowserTabCommand::Activate { target: target.clone() }).await.map_err(message)?;
        count(1)?;
        workspace.finish_run(&first).await.map_err(message)?;

        let second = workspace.begin_run().await.map_err(message)?;
        let observed = workspace.observe(&second, Some(target.tab_id.clone())).await.map_err(message)?;
        count(2)?; // A later Agent turn can open the same existing page again.
        let element = observed.elements.iter().find(|element| element.name == "Push SPA route").ok_or("Missing SPA fixture action")?.reference.clone();
        let bounds = BrowserSurfaceBounds { x: 10.0, y: 50.0, width: 900.0, height: 600.0 };
        workspace.set_surface(bounds, false, CancellationToken::new()).await.map_err(message)?;
        if native_visible(&view).await? { return Err("Hidden native controller unexpectedly became visible".into()); }
        let hidden_state = super::evaluate(&view, "({hidden:document.hidden,focused:document.hasFocus(),width:innerWidth,height:innerHeight,button:document.getElementById('spa').getBoundingClientRect().toJSON()})").await?;
        workspace.act(&second, BrowserAction::click(element)).await.map_err(|error|format!("Hidden native input: {error}; state={hidden_state}"))?;
        if native_visible(&view).await? || super::evaluate(&view, "location.pathname === '/spa-route' && window.spaClickTrusted === true").await? != true {
            return Err("Hidden Agent action did not deliver a trusted click without showing the controller".into());
        }
        count(2)?; // Hiding during this turn is not undone by each Agent action.
        let pointer = super::evaluate(&view, "(()=>{const nodes=document.querySelectorAll('[data-nomi-agent-pointer]'),el=nodes[0];if(!el)return {count:0};const s=getComputedStyle(el),r=el.getBoundingClientRect();return {count:nodes.length,passthrough:s.pointerEvents==='none',hidden:el.getAttribute('aria-hidden')==='true',closed:el.shadowRoot===null,isolated:window.__nomiPointer===undefined,hit:document.elementFromPoint(r.x+1,r.y+1)?.id}})()").await?;
        if pointer["count"]!=1 || pointer["passthrough"]!=true || pointer["hidden"]!=true || pointer["closed"]!=true || pointer["isolated"]!=true || pointer["hit"]!="spa" {
            return Err(format!("Agent pointer disturbed native page hit testing or isolation: {pointer}"));
        }
        let observed = workspace.observe(&second, Some(target.tab_id.clone())).await.map_err(message)?;
        if super::evaluate(&view, "document.querySelectorAll('[data-nomi-agent-pointer]').length").await? != 0 {
            return Err("Fresh observation leaked a previous pointer".into());
        }
        let input = observed.elements.iter().find(|element| element.name == "Background text input" && element.role == "textbox").ok_or("Missing background input")?.reference.clone();
        let input_state = super::evaluate(&view, "(()=>{const el=document.getElementById('background-input'),r=el.getBoundingClientRect();return {hidden:document.hidden,focused:document.hasFocus(),width:innerWidth,height:innerHeight,rect:r.toJSON(),hit:document.elementFromPoint(r.x+r.width/2,r.y+r.height/2)?.outerHTML}})()").await?;
        if let Err(error) = workspace.act(&second, BrowserAction::Type { element: input, text: "隐藏页面中文输入".into() }).await {
            // Diagnose the underlying scheduler only; do not inject an alternate
            // semantic core, replay input, or weaken the production stable check.
            let probe = "Promise.race([new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(()=>resolve('frames-running')))),new Promise(resolve=>setTimeout(()=>resolve('raf-timeout'),2500))])";
            let states = super::evaluate(&view, &probe).await?;
            return Err(format!("Hidden native text: {error}; state={input_state}; actionability={states}"));
        }
        let observed = workspace.observe(&second, Some(target.tab_id.clone())).await.map_err(message)?;
        let input = observed.elements.iter().find(|element| element.name == "Background text input" && element.role == "textbox").ok_or("Missing focused background input")?;
        if !input.focused { return Err("Hidden input did not retain its native page focus".into()); }
        workspace.act(&second, BrowserAction::Press { element: input.reference.clone(), keys: "End".into() }).await.map_err(message)?;
        let observed = workspace.observe(&second, Some(target.tab_id.clone())).await.map_err(message)?;
        let input = observed.elements.iter().find(|element| element.name == "Background text input" && element.role == "textbox").ok_or("Missing background input after End")?.reference.clone();
        workspace.act(&second, BrowserAction::Press { element: input, keys: "Backspace".into() }).await.map_err(message)?;
        let keyboard = super::evaluate(&view, "({value:document.getElementById('background-input').value,events:backgroundInputEvents})").await?;
        if native_visible(&view).await? || keyboard["value"] != "隐藏页面中文输" || !keyboard["events"].as_array().is_some_and(|events|
            !events.is_empty() && events.iter().all(|event| event["trusted"] == true) && events.iter().any(|event|event["type"]=="input") && events.iter().any(|event|event["key"]=="Backspace")) {
            return Err(format!("Hidden page did not receive trusted text and keyboard input: {keyboard}"));
        }
        count(2)?;
        super::evaluate(&view, "window.nativeMotionTimer=setInterval(()=>{document.getElementById('background-input').style.transform='translateX('+(performance.now()%1000)/10+'px)'},8);true").await?;
        let observed = workspace.observe(&second, Some(target.tab_id.clone())).await.map_err(message)?;
        let moving = observed.elements.iter().find(|element| element.name == "Background text input" && element.role == "textbox").ok_or("Missing moving background input")?.reference.clone();
        let moving_result = workspace.act(&second, BrowserAction::Type { element: moving, text: "must-not-be-entered".into() }).await;
        super::evaluate(&view, "clearInterval(nativeMotionTimer);document.getElementById('background-input').style.transform='none';true").await?;
        if !matches!(moving_result, Err(WorkspaceError::NotActionable)) || super::evaluate(&view, "document.getElementById('background-input').value").await? != "隐藏页面中文输" {
            return Err(format!("Moving background input was not rejected before mutation: {moving_result:?}"));
        }
        workspace.finish_run(&second).await.map_err(message)?;
        if super::evaluate(&view, "document.hidden").await? != true {
            return Err("Agent activation remained enabled after turn settlement".into());
        }
        if super::evaluate(&view, "popupNonce").await? != nonce {
            return Err("Presentation replaced the native page instance".into());
        }

        workspace.set_surface(bounds, true, CancellationToken::new()).await.map_err(message)?;
        let visible_run = workspace.begin_run().await.map_err(message)?;
        let observed=workspace.observe(&visible_run, Some(target.tab_id.clone())).await.map_err(message)?;
        let element=observed.elements.iter().find(|element|element.name=="Push SPA route").ok_or("Missing visible pointer fixture")?.reference.clone();
        workspace.act(&visible_run,BrowserAction::Hover {element}).await.map_err(message)?;
        if super::evaluate(&view, "document.querySelectorAll('[data-nomi-agent-pointer]').length").await? != 1 {
            return Err("Visible native action omitted its pointer".into());
        }
        let pointer_rect=super::evaluate(&view,"document.querySelector('[data-nomi-agent-pointer]').getBoundingClientRect().toJSON()").await?;
        let capture=workspace.screenshot(&visible_run,None).await.map_err(message)?;
        use base64::Engine;
        let bytes=base64::engine::general_purpose::STANDARD.decode(&capture.png_base64).map_err(message)?;
        let image=image::load_from_memory_with_format(&bytes,image::ImageFormat::Png).map_err(message)?.to_rgb8();
        let x=((pointer_rect["x"].as_f64().ok_or("Missing pointer x")?+5.0)*image.width() as f64/capture.viewport_width).round() as u32;
        let y=((pointer_rect["y"].as_f64().ok_or("Missing pointer y")?+12.0)*image.height() as f64/capture.viewport_height).round() as u32;
        let pixel=image.get_pixel_checked(x,y).ok_or("Pointer outside native capture")?.0;
        if pixel[0].abs_diff(112)>8 || pixel[1].abs_diff(101)>8 || pixel[2].abs_diff(238)>8 {
            return Err(format!("Native compositor did not paint the pointer at its input location: {pixel:?}"));
        }
        count(2)?; // An already visible page needs no layout/focus request.
        workspace.finish_run(&visible_run).await.map_err(message)?;
        if super::evaluate(&view, "document.querySelectorAll('[data-nomi-agent-pointer]').length").await? != 0 {
            return Err("Agent settlement left a pointer on the user's page".into());
        }
        let visible_run=workspace.begin_run().await.map_err(message)?;
        workspace.observe(&visible_run, Some(target.tab_id.clone())).await.map_err(message)?;
        workspace.set_surface(bounds, false, CancellationToken::new()).await.map_err(message)?;
        workspace.observe(&visible_run, Some(target.tab_id.clone())).await.map_err(message)?;
        count(2)?;
        workspace.finish_run(&visible_run).await.map_err(message)?;

        let cancelled = workspace.begin_run().await.map_err(message)?;
        // Keep this fixture visible so cancellation tests do not add an auto-open.
        workspace.set_surface(bounds, true, CancellationToken::new()).await.map_err(message)?;
        let observed=workspace.observe(&cancelled,None).await.map_err(message)?;
        let element=observed.elements.iter().find(|element|element.name=="Push SPA route").ok_or("Missing cancelled pointer fixture")?.reference.clone();
        workspace.act(&cancelled,BrowserAction::Hover {element}).await.map_err(message)?;
        cancelled.cancel();
        if workspace.observe(&cancelled, None).await.is_ok() {
            return Err("Cancelled run was allowed to observe".into());
        }
        count(2)?;
        workspace.finish_run(&cancelled).await.map_err(message)?;
        if super::evaluate(&view, "document.querySelectorAll('[data-nomi-agent-pointer]').length").await? != 0 {
            return Err("Cancelled Agent left a pointer on the user's page".into());
        }
        Ok(serde_json::json!({"observe_auto_opens_existing_page":true,"once_per_turn":true,
            "later_turn_can_open":true,"hide_is_respected_within_turn":true,"visible_page_does_not_reopen":true,
            "failed_and_cancelled_reads_do_not_open":true,"same_native_page":true,"hidden_page_input":true,"hidden_page_trusted_text_and_keys":true,"moving_hidden_input_rejected":true,"activation_restored_after_run":true,
            "pointer_passthrough_and_isolation":true,"native_pointer_pixels":true,"pointer_retired_on_observe_finish_and_cancel":true}))
    }.await;
    app.unlisten(listener);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match service.close(&key).await {
            Ok(()) => break,
            Err(error) if tokio::time::Instant::now() >= deadline => {
                return Err(format!("Presentation cleanup: {error}"));
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
        }
    }
    result
}
