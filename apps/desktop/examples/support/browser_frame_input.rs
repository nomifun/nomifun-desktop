//! The same native frame conformance cases exercise each independent host.
use super::{automation, set_fixture_browser_zoom};
use super::native::{self, View};
async fn evaluate(view: &View, expression: &str) -> Result<serde_json::Value, String> {
    let result = native::protocol_call(view, "Runtime.evaluate", serde_json::json!({"expression":expression,"returnByValue":true,"awaitPromise":true})).await?;
    if result.get("exceptionDetails").is_some() { return Err("Native frame fixture evaluation failed".into()); }
    Ok(result["result"]["value"].clone())
}
pub(super) async fn frame_element(
    driver: &mut automation::TabAutomation,
    view: &View,
    name: &str,
) -> Result<nomifun_browser_platform::runtime::BrowserElementRef, String> {
    use nomifun_browser_platform::runtime::BrowserTabTarget;
    let observation = driver
        .observe(
            view,
            BrowserTabTarget {
                tab_id: "browser-smoke".into(),
                runtime_generation: 1,
                document_generation: 1,
            },
            &tokio_util::sync::CancellationToken::new(),
        )
        .await
        .map_err(|error| error.to_string())?;
    if observation.unobserved_frames != 0 {
        return Err(format!(
            "Frame observation left {} frames unread",
            observation.unobserved_frames
        ));
    }
    let reference = observation
        .elements
        .iter()
        .find(|element| element.name == name)
        .ok_or_else(|| format!("Frame observation omitted {name}: {}", observation.content))?
        .reference
        .clone();
    if !observation
        .content
        .contains(&format!("[ref={}]", reference.ref_id))
    {
        return Err("Aria text and structured frame refs disagree.".into());
    }
    Ok(reference)
}

pub(super) async fn verify_frame_input(view: &View, url: &str) -> Result<serde_json::Value, String> {
    use nomifun_browser_platform::runtime::{BrowserAction, WorkspaceError};
    use serde_json::json;
    let cancel = tokio_util::sync::CancellationToken::new();
    let mut driver = automation::TabAutomation::default();
    driver
        .initialize_frames(view)
        .await
        .map_err(|error| error.to_string())?;
    native::protocol_call(view, "Page.navigate", json!({"url":format!("{url}?input")})).await?;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if evaluate(
            view,
            "document.readyState === 'complete' && !!document.getElementById('foreign')",
        )
        .await?
            == true
        {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            let state = evaluate(view, "({url:location.href,ready:document.readyState,foreign:!!document.getElementById('foreign'),body:document.body?.innerHTML.slice(0,600)})").await?;
            return Err(format!("Frame input fixture did not load: {state}"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    native::set_user_input_enabled(view, false).await?;
    for level in ["Cross-site", "Nested", "Same-process"] {
        let element = frame_element(&mut driver, view, &format!("{level} button")).await?;
        driver
            .act(view, BrowserAction::click(element), &cancel)
            .await
            .map_err(|error| format!("{level} native click: {error}"))?;
        let element = frame_element(&mut driver, view, &format!("{level} text")).await?;
        driver
            .act(
                view,
                BrowserAction::Type {
                    element,
                    text: format!("{level} 中文"),
                },
                &cancel,
            )
            .await
            .map_err(|error| format!("{level} native type: {error}"))?;
        let element = frame_element(&mut driver, view, &format!("{level} text")).await?;
        driver
            .act(
                view,
                BrowserAction::Press {
                    element,
                    keys: "End".into(),
                },
                &cancel,
            )
            .await
            .map_err(|error| format!("{level} native key: {error}"))?;
        let element = frame_element(&mut driver, view, &format!("{level} choice")).await?;
        if let Err(error) = driver.act(view, BrowserAction::Select { element, labels: vec!["Two".into()] }, &cancel).await {
            let evidence = evaluate(view, "frameEvidence").await?;
            return Err(format!("{level} native select: {error}; fixture events: {evidence}"));
        }
    }
    // postMessage evidence arrives on the parent's event queue, independently
    // of the completed input command in the child process.
    let delivered = evaluate(view, r#"new Promise(resolve=>{
        const ready=()=>['Cross-site','Nested','Same-process'].every(level=>window.frameEvidence.some(
            event=>event.nomiFrame===level && event.type==='change' && event.target==='choice' && event.value==='Two'));
        if (ready()) { resolve(true); return; }
        const done=value=>{removeEventListener('message',listener);clearTimeout(timer);resolve(value)};
        const listener=()=>{if(ready())done(true)};
        const timer=setTimeout(()=>done(false),2000);
        addEventListener('message',listener);
    })"#).await?;
    if delivered != true {
        return Err("Frame input evidence did not reach the parent.".into());
    }
    let evidence = evaluate(view, "window.frameEvidence").await?;
    let events = evidence
        .as_array()
        .ok_or("Frame input evidence was absent")?;
    for level in ["Cross-site", "Nested", "Same-process"] {
        for (kind, target) in [
            ("click", "button"),
            ("input", "field"),
            ("keydown", "field"),
            ("change", "choice"),
        ] {
            if !events.iter().any(|event| {
                event["nomiFrame"] == level
                    && event["type"] == kind
                    && event["target"] == target
                    && event["trusted"] == true
            }) {
                return Err(format!(
                    "{level} did not receive trusted {kind} on {target}: {evidence}"
                ));
            }
        }
        if !events.iter().any(|event| {
            event["nomiFrame"] == level
                && event["type"] == "input"
                && event["value"] == format!("{level} 中文")
        }) {
            return Err(format!("{level} did not receive its native text"));
        }
        if !events.iter().any(|event| {
            event["nomiFrame"] == level && event["target"] == "choice" && event["value"] == "Two"
        }) {
            return Err(format!(
                "{level} native selection did not change the option"
            ));
        }
    }
    let mut individual_transforms = vec![];
    for (rotate, scale, translate) in [
        ("7deg", "80% 105%", "13px 9px"),
        ("0 0 -1 5deg", "-0.8 0.9", "470px 12px"),
    ] {
        evaluate(view,&format!("(()=>{{const frame=document.getElementById('foreign');frame.style.rotate={rotate:?};frame.style.scale={scale:?};frame.style.translate={translate:?};window.frameEvidence=[];return true}})()")).await?;
        let element = frame_element(&mut driver, view, "Nested button").await?;
        driver
            .act(view, BrowserAction::click(element), &cancel)
            .await
            .map_err(|error| format!("Individual iframe transform {rotate}/{scale}: {error}"))?;
        let received=evaluate(view,r#"new Promise(resolve=>{
            const ready=()=>frameEvidence.some(event=>event.nomiFrame==='Nested' && event.target==='button' && event.type==='click' && event.trusted);
            if(ready()){resolve(true);return}
            const finish=value=>{clearTimeout(timer);removeEventListener('message',listener);resolve(value)};
            const listener=()=>{if(ready())finish(true)};
            const timer=setTimeout(()=>finish(false),1000);addEventListener('message',listener);
        })"#).await?;
        if received != true {
            return Err("Individual transform click missed the nested control".into());
        }
        individual_transforms.push(
            json!({"rotate":rotate,"scale":scale,"translate":translate,"trusted_click":true}),
        );
    }
    evaluate(view,"(()=>{const style=document.getElementById('foreign').style;style.removeProperty('rotate');style.removeProperty('scale');style.removeProperty('translate');return true})()").await?;
    // A child retains its own activeElement after focus leaves its iframe.
    // Press must check every parent, not merely the child's activeElement.
    let element = frame_element(&mut driver, view, "Nested text").await?;
    driver
        .act(
            view,
            BrowserAction::Type {
                element,
                text: "focus guard".into(),
            },
            &cancel,
        )
        .await
        .map_err(|error| error.to_string())?;
    let element = frame_element(&mut driver, view, "Nested text").await?;
    evaluate(view, "document.getElementById('root').focus();true").await?;
    if !matches!(
        driver
            .act(
                view,
                BrowserAction::Press {
                    element,
                    keys: "Enter".into()
                },
                &cancel
            )
            .await,
        Err(WorkspaceError::NotActionable)
    ) {
        return Err("Keyboard bypassed the iframe ancestor focus guard.".into());
    }
    let element = frame_element(&mut driver, view, "Nested button").await?;
    evaluate(view, "(()=>{const cover=document.createElement('div');cover.id='cover';cover.style.cssText='position:fixed;inset:0;z-index:999999';document.body.append(cover);return true})()").await?;
    if !matches!(
        driver
            .act(view, BrowserAction::click(element), &cancel)
            .await,
        Err(WorkspaceError::NotActionable)
    ) {
        return Err("Native input crossed a parent-frame overlay.".into());
    }
    evaluate(view, "document.getElementById('cover').remove();true").await?;
    let element = frame_element(&mut driver, view, "Same-process text").await?;
    evaluate(view, "(()=>{const doc=document.getElementById('same').contentDocument;doc.getElementById('field').addEventListener('keydown',event=>{if((event.ctrlKey||event.metaKey)&&event.key==='a'){event.preventDefault();doc.getElementById('choice').focus()}});return true})()").await?;
    if !matches!(
        driver
            .act(
                view,
                BrowserAction::Type {
                    element,
                    text: "must-not-insert".into()
                },
                &cancel
            )
            .await,
        Err(WorkspaceError::ActionInterrupted)
    ) {
        return Err("Text insertion ignored focus redirected by a key handler.".into());
    }
    if evaluate(
        view,
        "document.getElementById('same').contentDocument.getElementById('field').value",
    )
    .await?
        != "Same-process 中文"
    {
        return Err("Focus-interrupted typing changed the field.".into());
    }
    let element = frame_element(&mut driver, view, "Nested button").await?;
    evaluate(view, "new Promise(resolve=>{const frame=document.getElementById('foreign');frame.onload=()=>resolve(true);frame.src=frame.src+'?reload'})").await?;
    if !matches!(
        driver
            .act(view, BrowserAction::click(element), &cancel)
            .await,
        Err(WorkspaceError::StaleObservation)
    ) {
        return Err("A reference survived its ancestor iframe navigation.".into());
    }
    let element = frame_element(&mut driver, view, "Nested button").await?;
    driver
        .act(view, BrowserAction::click(element), &cancel)
        .await
        .map_err(|error| error.to_string())?;
    for (style,perspective) in [
        ("transform:perspective(500px) rotateY(25deg)","none"),
        ("transform:rotateY(25deg)","800px"),
        ("transform:perspective(600px) rotateX(10deg);rotate:y 20deg;scale:.9 .95 1.2;translate:10px 0 30px","none"),
        ("transform:perspective(500px) rotateY(15deg);offset-path:path('M 300 180 L 340 200');offset-distance:50%;offset-rotate:0deg","none"),
    ] {
        evaluate(view,&format!("document.body.style.perspective={perspective:?};document.getElementById('foreign').style.cssText={style:?};window.frameEvidence=[];true")).await?;
        let element=frame_element(&mut driver,view,"Nested button").await?;
        driver.act(view,BrowserAction::click(element),&cancel).await.map_err(|error|format!("Projected iframe {style}: {error}"))?;
        wait_trusted_frame_event(view,"Nested","click","button").await?;
    }
    evaluate(view,"document.body.style.perspective='none';document.getElementById('foreign').style.cssText='';true").await?;
    evaluate(view,r#"new Promise(resolve=>{
        const parent=document.getElementById('same');parent.style.transform='perspective(500px) rotateY(18deg)';
        const frame=parent.contentDocument.createElement('iframe');frame.id='same-nested';frame.src='/frame-same-nested';
        frame.style.cssText='position:fixed;left:20px;top:8px;margin:0;width:220px;height:110px;transform:perspective(400px) rotateY(-15deg);transform-origin:top left';
        frame.onload=()=>resolve(true);parent.contentDocument.body.append(frame);
    })"#).await?;
    evaluate(view,"window.frameEvidence=[];true").await?;
    let element=frame_element(&mut driver,view,"Same-nested button").await?;
    driver.act(view,BrowserAction::click(element),&cancel).await.map_err(|error|format!("Same-process nested perspective: {error}"))?;
    wait_trusted_frame_event(view,"Same-nested","click","button").await?;
    let element=frame_element(&mut driver,view,"Same-nested text").await?;
    driver.act(view,BrowserAction::Type {element,text:"Perspective 中文".into()},&cancel).await.map_err(|error|format!("Perspective typing: {error}"))?;
    if evaluate(view,"document.getElementById('same').contentDocument.getElementById('same-nested').contentDocument.getElementById('field').value").await?!="Perspective 中文" {
        return Err("Perspective typing did not reach the nested input".into());
    }
    // Two postMessage hops deliver the native event asynchronously, after the
    // input value itself has changed. Await the witness rather than dropping it.
    wait_trusted_frame_event(view,"Same-nested","input","field").await?;
    let element=frame_element(&mut driver,view,"Same-nested button").await?;
    evaluate(view,"(()=>{const doc=document.getElementById('same').contentDocument;const cover=doc.createElement('div');cover.id='perspective-cover';cover.style.cssText='position:fixed;inset:0;z-index:999999';doc.body.append(cover);return true})()").await?;
    if !matches!(driver.act(view,BrowserAction::click(element),&cancel).await,Err(WorkspaceError::NotActionable)) {
        return Err("Projected input bypassed an overlay inside its parent document".into());
    }
    evaluate(view,"document.getElementById('same').contentDocument.getElementById('perspective-cover').remove();true").await?;
    // Native browser zoom, not device/phone emulation or CSS zoom.
    evaluate(view,"document.getElementById('foreign').style.visibility='hidden';document.getElementById('same').style.top='60px';document.getElementById('root').style.cssText='position:fixed;left:500px;top:200px;width:80px;height:30px';document.getElementById('root').onclick=event=>window.rootZoomClick=event.isTrusted;true").await?;
    let unzoomed=evaluate(view,"innerWidth").await?.as_f64().ok_or("Missing CSS viewport")?;
    let mut zoom_evidence=vec![];
    for zoom in [0.8,1.25,1.5] {
        set_fixture_browser_zoom(view,zoom).await?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(2);
        let css_width=loop {
            let actual=evaluate(view,"innerWidth").await?.as_f64().ok_or("Missing zoomed viewport")?;
            if (actual-unzoomed/zoom).abs()<2. {break actual;}
            if tokio::time::Instant::now()>=deadline {return Err(format!("Native zoom {zoom} did not reach its expected CSS viewport: {actual}"));}
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        };
        evaluate(view,"window.frameEvidence=[];window.rootZoomClick=false;true").await?;
        let root=frame_element(&mut driver,view,"Root button").await?;
        driver.act(view,BrowserAction::click(root),&cancel).await.map_err(|error|format!("Root click at zoom {zoom}: {error}"))?;
        if evaluate(view,"rootZoomClick").await?!=true {return Err("Zoomed root input missed its far-positioned button".into());}
        let element=frame_element(&mut driver,view,"Same-nested button").await?;
        driver.act(view,BrowserAction::click(element),&cancel).await.map_err(|error|format!("Browser zoom {zoom}: {error}"))?;
        wait_trusted_frame_event(view,"Same-nested","click","button").await?;
        let element=frame_element(&mut driver,view,"Same-nested text").await?;
        let typed=format!("zoom {zoom} 中文");
        driver.act(view,BrowserAction::Type {element,text:typed.clone()},&cancel).await.map_err(|error|format!("Zoomed typing {zoom}: {error}"))?;
        wait_trusted_frame_event(view,"Same-nested","input","field").await?;
        if evaluate(view,"document.getElementById('same').contentDocument.getElementById('same-nested').contentDocument.getElementById('field').value").await?.as_str()!=Some(typed.as_str()) {return Err("Zoomed typing reached the wrong field".into());}
        zoom_evidence.push(json!({"factor":zoom,"css_width":css_width,"root_click":true,"nested_click":true,"trusted_typing":true}));
    }
    // Root scrolling must not be confused with document-space box coordinates.
    evaluate(view,"document.body.style.minHeight='2000px';document.getElementById('same').style.top='1000px';window.scrollTo(0,950);window.frameEvidence=[];true").await?;
    let element=frame_element(&mut driver,view,"Same-nested button").await?;
    driver.act(view,BrowserAction::click(element),&cancel).await.map_err(|error|format!("Scrolled zoomed iframe: {error}"))?;
    wait_trusted_frame_event(view,"Same-nested","click","button").await?;
    if evaluate(view,"scrollY").await?.as_f64().unwrap_or(0.)<500. {return Err("The scroll-coordinate case did not retain a scrolled root viewport".into());}
    set_fixture_browser_zoom(view,1.).await?;
    driver
        .release(view)
        .await
        .map_err(|error| error.to_string())?;
    native::set_user_input_enabled(view, true).await?;
    Ok(
        json!({"trusted_events":evidence,"same_process_and_oopif":true,"nested_affine_mapping":true,"individual_transforms":individual_transforms,
            "parent_focus_guard":true,"key_redirect_stops_text":true,"parent_overlay_rejected":true,"ancestor_navigation_stales_refs":true,"perspective_click":true,"ancestor_perspective_click":true,"individual_3d_transform":true,"motion_path_click":true,"same_process_nested_perspective":true,"perspective_typing":true,"perspective_parent_overlay_rejected":true,"native_zoom_projected_input":zoom_evidence,"root_scroll_after_zoom":true}),
    )
}

async fn wait_trusted_frame_event(view:&View,level:&str,kind:&str,target:&str)->Result<(),String> {
    let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(1);
    loop {
        let events=evaluate(view,"frameEvidence").await?;
        if events.as_array().is_some_and(|events|events.iter().any(|event|event["nomiFrame"]==level&&event["target"]==target&&event["type"]==kind&&event["trusted"]==true)) {return Ok(());}
        if tokio::time::Instant::now()>=deadline {return Err(format!("Projected native {kind} did not reach {level}/{target}: {events}"));}
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    }
}
