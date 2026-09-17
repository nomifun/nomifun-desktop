use nomi_tools::{Tool, registry::ToolRegistry};
use nomifun_agent_contracts::ActionId;
use nomifun_browser_platform::{
    bound_resource::BoundBrowserProviderResource,
    product::BrowserProviderKind,
    run_guard::BrowserInputState,
    runtime::*,
    workspace::BrowserResourceService,
};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::Manager;

fn message(error: impl std::fmt::Display) -> String { error.to_string() }
async fn native_visibility(view: &tauri::Webview) -> Result<(bool,bool),String> {
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result=(|| -> windows::core::Result<(bool,bool)> {
            let mut visible=windows::core::BOOL::default();
            let mut container=windows::Win32::Foundation::HWND::default();
            unsafe {
                platform.controller().IsVisible(&mut visible)?;
                platform.controller().ParentWindow(&mut container)?;
                Ok((visible.as_bool(),windows::Win32::UI::WindowsAndMessaging::IsWindowVisible(container).as_bool()))
            }
        })().map_err(message);
        let _=tx.send(result);
    }).map_err(message)?;
    rx.await.map_err(message)?
}
async fn zoom_factor(view: &tauri::Webview, value: Option<f64>) -> Result<f64,String> {
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result=(|| -> windows::core::Result<f64> {
            let mut previous=0.0;
            unsafe {
                platform.controller().ZoomFactor(&mut previous)?;
                if let Some(value)=value { platform.controller().SetZoomFactor(value)?; }
            }
            Ok(previous)
        })().map_err(message);
        let _=tx.send(result);
    }).map_err(message)?;
    rx.await.map_err(message)?
}
fn capabilities(ids: &[&str]) -> std::collections::BTreeSet<ActionId> {
    ids.iter().map(|id| (*id).into()).collect()
}

async fn scaled_captures(view: &tauri::Webview, turn: &super::browser_lifecycle::ManagedBrowserTurn) -> Result<(),String> {
    use base64::Engine;
    let result=async {
        for density in [1.0,2.0,3.0] {
            super::windows::protocol_call(view,"Emulation.setDeviceMetricsOverride",json!({"width":1200,"height":800,"deviceScaleFactor":density,"mobile":false})).await?;
            let before=super::evaluate(view,"({width:innerWidth,height:innerHeight,density:devicePixelRatio,nonce:popupNonce})").await?;
            let metrics=super::windows::protocol_call(view,"Page.getLayoutMetrics",json!({})).await?;
            super::evaluate(view,"window.densityReads=0;window.originalDensityDescriptor=Object.getOwnPropertyDescriptor(window,'devicePixelRatio');Object.defineProperty(window,'devicePixelRatio',{get(){densityReads++;return 0},configurable:true});true").await?;
            let captured=turn.screenshot(None).await.map_err(|error|format!("Screenshot density {density}: {error}; metrics={metrics}"))?;
            if super::evaluate(view,"densityReads").await?!=0 { return Err("Screenshot trusted a page-supplied pixel density getter".into()); }
            super::evaluate(view,"Object.defineProperty(window,'devicePixelRatio',originalDensityDescriptor);delete window.originalDensityDescriptor;true").await?;
            let bytes=base64::engine::general_purpose::STANDARD.decode(&captured.png_base64).map_err(message)?;
            let image=image::load_from_memory_with_format(&bytes,image::ImageFormat::Png).map_err(message)?.to_rgb8();
            if image.width()>1600 || image.height()>1600 || image.width()<1000 || captured.viewport_width!=1200.0 || captured.viewport_height!=800.0 {
                return Err(format!("Unexpected scaled capture at density {density}: {}x{} viewport={}x{}",image.width(),image.height(),captured.viewport_width,captured.viewport_height));
            }
            for color in [[224,32,64],[32,112,224]] {
                if image.pixels().filter(|pixel|pixel.0==color).count()<300 { return Err(format!("Scaled capture lost Canvas color at density {density}")); }
            }
            if super::evaluate(view,"({width:innerWidth,height:innerHeight,density:devicePixelRatio,nonce:popupNonce})").await?!=before {
                return Err("Screenshot resized or replaced the page".into());
            }
        }
        super::windows::protocol_call(view,"Emulation.clearDeviceMetricsOverride",json!({})).await?;
        turn.screenshot(None).await.map_err(message)?;
        let original_zoom=zoom_factor(view,None).await?;
        let base_density=super::evaluate(view,"devicePixelRatio").await?.as_f64().ok_or("Missing base pixel density")?;
        let zoomed=async {
            for zoom in [0.8,1.25,2.0] {
                zoom_factor(view,Some(zoom)).await?;
                let expected_density=base_density*zoom/original_zoom;
                let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(2);
                loop {
                    let density=super::evaluate(view,"devicePixelRatio").await?.as_f64().ok_or("Missing zoomed pixel density")?;
                    if (density-expected_density).abs()<1e-6 { break; }
                    if tokio::time::Instant::now()>=deadline { return Err("Native zoom did not reach the page".into()); }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                let before=super::evaluate(view,"({width:innerWidth,height:innerHeight,density:devicePixelRatio,nonce:popupNonce})").await?;
                super::evaluate(view,"document.getElementById('visual-probe').style.position='fixed';document.body.style.minHeight='3000px';window.scrollTo(0,180);true").await?;
                let scroll=super::evaluate(view,"scrollY").await?;
                if scroll.as_f64().unwrap_or(0.0)<=0.0 { return Err("Scroll capture fixture did not scroll".into()); }
                let captured=turn.screenshot(None).await.map_err(|error|format!("Screenshot zoom {zoom}: {error}"))?;
                let bytes=base64::engine::general_purpose::STANDARD.decode(&captured.png_base64).map_err(message)?;
                let image=image::load_from_memory_with_format(&bytes,image::ImageFormat::Png).map_err(message)?.to_rgb8();
                if image.width()>1600 || image.height()>1600 || !image.pixels().any(|pixel|pixel.0==[224,32,64]) || !image.pixels().any(|pixel|pixel.0==[32,112,224]) {
                    let red=image.pixels().filter(|pixel|pixel.0==[224,32,64]).count();
                    let blue=image.pixels().filter(|pixel|pixel.0==[32,112,224]).count();
                    return Err(format!("Zoom {zoom} lost viewport Canvas pixels: {}x{}, red={red}, blue={blue}, before={before}, metrics={}",image.width(),image.height(),super::windows::protocol_call(view,"Page.getLayoutMetrics",json!({})).await?));
                }
                let after=super::evaluate(view,"({width:innerWidth,height:innerHeight,density:devicePixelRatio,nonce:popupNonce})").await?;
                let actual_zoom=zoom_factor(view,None).await?;
                if (actual_zoom-zoom).abs()>1e-8 || after!=before || super::evaluate(view,"scrollY").await?!=scroll {
                    return Err(format!("Screenshot changed browser zoom or layout: requested={zoom} actual={actual_zoom} before={before} after={after}"));
                }
            }
            Ok::<_,String>(())
        }.await;
        zoom_factor(view,Some(original_zoom)).await?;
        super::evaluate(view,"document.body.style.minHeight='';window.scrollTo(0,0);true").await?;
        zoomed?;
        Ok(())
    }.await;
    super::windows::protocol_call(view,"Emulation.clearDeviceMetricsOverride",json!({})).await?;
    result
}
async fn invoke(registry: &ToolRegistry, input: Value) -> Result<Value, String> {
    registry.validate_input("Browser", &input)?;
    let tool = registry.get("Browser").ok_or("Native Browser tool not registered")?;
    let result = tool.execute(input).await;
    if result.is_error { return Err(format!("Native Browser tool: {}", result.content)); }
    serde_json::from_str(&result.content).map_err(message)
}
async fn reject(tool: &impl Tool, input: Value, code: &str) -> Result<(), String> {
    let result = tool.execute(input).await;
    let body: Value = serde_json::from_str(&result.content).map_err(message)?;
    if !result.is_error || body["code"] != code { return Err(format!("Expected {code}, got {}", result.content)); }
    Ok(())
}
fn element(observation: &Value, name: &str, role: &str) -> Result<Value, String> {
    observation["elements"].as_array().and_then(|elements|elements.iter().find(|element|element["name"]==name && element["role"]==role))
        .map(|element| element["reference"].clone()).ok_or_else(||format!("Tool observation omitted {name}"))
}

pub(super) async fn verify(app: &tauri::AppHandle, url: &str) -> Result<Value, String> {
    let service = BrowserResourceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let authority = super::browser_resource_fixture::authority("tool-fixture", "tool-fixture", "native-tool-fixture-provider");
    let key = authority.key();
    let workspace = service.ensure(authority, BrowserProfile::Ephemeral).await.map_err(message)?;
    let bound = BoundBrowserProviderResource::Managed(workspace.clone());
    let slot = super::browser_lifecycle::BrowserTurnSlot::default();
    let tool = super::browser_tool::ConversationBrowserTool::new(slot.clone(), capabilities(&["browser/observe","browser/act","browser/navigate"]), BrowserProviderKind::Managed);
    let reader = super::browser_tool::ConversationBrowserTool::new(slot.clone(), capabilities(&["browser/observe"]), BrowserProviderKind::Managed);
    let mut registry = ToolRegistry::new();
    if !registry.register(Box::new(super::browser_tool::ConversationBrowserTool::new(slot.clone(), capabilities(&["browser/observe","browser/act","browser/navigate"]), BrowserProviderKind::Managed))) {
        return Err("Native Browser registry registration failed".into());
    }
    let result = async {
        reject(&tool,json!({"operation":"tabs"}),"BROWSER_STALE_RUN").await?;
        slot.begin(&bound).await.map_err(message)?;
        let super::browser_lifecycle::BrowserTurn::Managed(old_turn) = slot.current().map_err(message)? else { return Err("Expected managed Browser turn".into()); };
        let navigated = invoke(&registry,json!({"operation":"navigate","url":url})).await?;
        let target = super::wait_workspace_page(&workspace,url).await?;
        if navigated["active_tab_id"] != target.tab_id { return Err("Tool navigation used another native tab".into()); }
        let view = app.get_webview(&target.tab_id).ok_or("Missing tool-owned native page")?;
        let nonce = super::evaluate(&view,"popupNonce").await?;
        if native_visibility(&view).await?!=(false,false) { return Err("Screenshot fixture must begin with a hidden native child".into()); }
        registry.validate_input("Browser",&json!({"operation":"screenshot"}))?;
        let screenshot = registry.get("Browser").unwrap().execute(json!({"operation":"screenshot"})).await;
        if screenshot.is_error || screenshot.images.len()!=1 || screenshot.images[0].media_type!="image/png" {
            return Err(format!("Native screenshot tool failed: {}",screenshot.content));
        }
        let screenshot_meta:Value=serde_json::from_str(&screenshot.content).map_err(message)?;
        if screenshot_meta["target"]["tab_id"]!=target.tab_id || screenshot_meta["untrusted_page_content"]!=true { return Err("Screenshot metadata is not bound to the native page".into()); }
        use base64::Engine;
        let bytes=base64::engine::general_purpose::STANDARD.decode(&screenshot.images[0].data).map_err(message)?;
        let decoded=image::load_from_memory_with_format(&bytes,image::ImageFormat::Png).map_err(message)?.to_rgb8();
        if screenshot_meta["width"]!=decoded.width() || screenshot_meta["height"]!=decoded.height() { return Err("Screenshot dimensions disagree with the PNG".into()); }
        let red=decoded.pixels().filter(|pixel|pixel.0==[224,32,64]).count();
        let blue=decoded.pixels().filter(|pixel|pixel.0==[32,112,224]).count();
        if red<300 || blue<300 { return Err(format!("Screenshot omitted the native Canvas pixels: red={red}, blue={blue}")); }
        if native_visibility(&view).await?!=(false,false) { return Err("Screenshot exposed the hidden native child or retained compositor visibility".into()); }
        scaled_captures(&view,&old_turn).await?;
        let busy=super::evaluate(&view,"(()=>{const end=performance.now()+300;while(performance.now()<end){};return true})()");
        let capture=old_turn.screenshot(None);
        let hide=async {
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
            let visibility=native_visibility(&view).await?;
            if visibility!=(true,false) { return Err(format!("Expected an in-flight capture with hidden HWND, got {visibility:?}")); }
            tokio::time::timeout(std::time::Duration::from_millis(200),workspace.set_surface(BrowserSurfaceBounds {x:10.0,y:50.0,width:900.0,height:600.0},false,tokio_util::sync::CancellationToken::new()))
                .await.map_err(|_|"Hide waited for screenshot completion".to_owned())?.map_err(message)?;
            if native_visibility(&view).await?.1 { return Err("Capture hide exposed the native HWND".into()); }
            Ok::<_,String>(())
        };
        let (busy,captured,hidden)=tokio::join!(busy,capture,hide);
        busy?; captured.map_err(message)?; hidden?;
        if native_visibility(&view).await?!=(false,false) { return Err("Capture cleanup did not preserve the latest hide".into()); }
        let no_observe = super::browser_tool::ConversationBrowserTool::new(slot.clone(), capabilities(&["browser/act"]), BrowserProviderKind::Managed);
        reject(&no_observe,json!({"operation":"screenshot"}),"CAPABILITY_NOT_SELECTED").await?;
        if workspace.snapshot().await.map_err(message)?.run.input_state != BrowserInputState::AgentRunning {
            return Err("Tool page was not locked during the Agent run".into());
        }
        if workspace.user_command(BrowserTabCommand::Reload { target: target.clone() }).await.is_ok() {
            return Err("User navigation entered the running tool workspace".into());
        }
        reject(&reader,json!({"operation":"navigate","url":"http://localhost:1/"}),"CAPABILITY_NOT_SELECTED").await?;
        reject(&tool,json!({"operation":"act","action":{"action":"click","element":{"target":target,"observation_generation":0,"ref_id":"forged"}}}),"BROWSER_STALE_OBSERVATION").await?;
        let observed = invoke(&registry,json!({"operation":"observe"})).await?;
        let input = element(&observed,"Background text input","textbox")?;
        invoke(&registry,json!({"operation":"act","action":{"action":"type","element":input,"text":"Tool 中文回路"}})).await?;
        reject(&tool,json!({"operation":"act","action":{"action":"click","element":input}}),"BROWSER_STALE_OBSERVATION").await?;
        let observed = invoke(&registry,json!({"operation":"observe"})).await?;
        let button = element(&observed,"Produce browser diagnostics","button")?;
        invoke(&registry,json!({"operation":"act","action":{"action":"click","element":button}})).await?;
        let deadline = tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        loop {
            let diagnostics = invoke(&registry,json!({"operation":"diagnostics"})).await?;
            if diagnostics["untrusted_page_content"] != true { return Err("Tool diagnostics lack their untrusted marker".into()); }
            if diagnostics["diagnostics"]["entries"].as_array().is_some_and(|entries|entries.iter().any(|entry|entry["message"].as_str().is_some_and(|message|message.contains("nomi-native-console")))) { break; }
            if tokio::time::Instant::now()>=deadline { return Err("Tool did not receive native diagnostic events".into()); }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let proof = super::evaluate(&view,"({value:document.getElementById('background-input').value,events:backgroundInputEvents,nonce:popupNonce})").await?;
        if proof["value"]!="Tool 中文回路" || proof["nonce"]!=nonce || !proof["events"].as_array().is_some_and(|events|events.iter().any(|event|event["type"]=="input" && event["trusted"]==true)) {
            return Err(format!("Tool did not mutate the same page through trusted input: {proof}"));
        }
        let busy=super::evaluate(&view,"(()=>{const end=performance.now()+300;while(performance.now()<end){};return true})()");
        let capture=old_turn.screenshot(None);
        let cancel_capture=async { tokio::time::sleep(std::time::Duration::from_millis(30)).await; slot.cancel(); };
        let (busy,captured,())=tokio::join!(busy,capture,cancel_capture);
        busy?;
        if !matches!(captured,Err(WorkspaceError::Admission(nomifun_browser_platform::run_guard::RunAdmissionError::Cancelled))) || native_visibility(&view).await?!=(false,false) {
            return Err("Cancelled screenshot published data or retained native rendering".into());
        }
        slot.settle().await.map_err(message)?;
        if workspace.snapshot().await.map_err(message)?.run.input_state != BrowserInputState::AgentRunning { return Err("Settle unlocked before the terminal boundary".into()); }
        reject(&tool,json!({"operation":"tabs"}),"BROWSER_OPERATION_CANCELLED").await?;
        reject(&tool,json!({"operation":"screenshot"}),"BROWSER_OPERATION_CANCELLED").await?;
        slot.finish().await.map_err(message)?;
        reject(&tool,json!({"operation":"tabs"}),"BROWSER_STALE_RUN").await?;
        if workspace.snapshot().await.map_err(message)?.run.input_state!=BrowserInputState::UserReady { return Err("Terminal finish did not restore user input".into()); }
        slot.begin(&bound).await.map_err(message)?;
        if old_turn.tabs().await.is_ok() { return Err("Old tool invocation adopted the next run".into()); }
        let observed = invoke(&registry,json!({"operation":"observe"})).await?;
        if observed["target"]["tab_id"]!=target.tab_id || super::evaluate(&view,"popupNonce").await?!=nonce { return Err("The next tool run replaced the native page".into()); }
        slot.finish().await.map_err(message)?;
        Ok(json!({"production_tool_json_roundtrip":true,"same_native_page":true,"trusted_text_input":true,"native_diagnostics":true,"native_png_canvas_observation":true,"screenshot_density_and_zoom":true,"screenshot_ignores_page_density_getter":true,"screenshot_hide_race":true,"screenshot_cancel_cleanup":true,
            "capability_and_stale_refs_enforced":true,"settle_before_unlock":true,"old_turn_rejected":true}))
    }.await;
    slot.cancel();
    let _ = slot.settle().await;
    let _ = slot.finish().await;
    let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
    loop {
        match service.close(&key).await {
            Ok(())=>break,
            Err(error) if tokio::time::Instant::now()>=deadline=>return Err(format!("Native tool cleanup: {error}")),
            Err(_)=>tokio::time::sleep(std::time::Duration::from_millis(25)).await,
        }
    }
    result
}
