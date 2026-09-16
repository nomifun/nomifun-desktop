//! File protocol conformance on real same-process and out-of-process frames.
//! Application/factory/run integration is covered by --agent-only separately.
use super::native::{self, View};
use nomifun_browser_platform::uploads::BrowserUploadScope;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(super) async fn verify(view: &View, url: &str) -> Result<Value, String> {
    let root = tempfile::tempdir().map_err(|error| error.to_string())?;
    let name = "附件.txt";
    let payload = "Native iframe upload 中文";
    std::fs::write(root.path().join(name), payload).map_err(|error| error.to_string())?;
    let scope = Arc::new(BrowserUploadScope::open(root.path()).map_err(|error| error.to_string())?);
    let cancel = CancellationToken::new();
    let files = scope
        .prepare(vec![name.into()], cancel.clone())
        .await
        .map_err(|error| error.to_string())?;
    std::fs::write(root.path().join(name), "changed after snapshot")
        .map_err(|error| error.to_string())?;
    let mut driver = super::automation::TabAutomation::default();
    let result=async {
        // Establish ownership before navigation, but perform no observation or
        // frame-tree refresh until the new OOPIF's first chooser is verified.
        driver.initialize_frames(view).await.map_err(|error|error.to_string())?;
        native::protocol_call(view,"Page.navigate",json!({"url":url})).await?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
        loop {
            if evaluate(view,"window.frameReady?.length===2").await?==true {break;}
            if tokio::time::Instant::now()>=deadline {return Err("File frame fixtures did not load".into());}
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        native::set_user_input_enabled(view,false).await?;
        let mut early=native::file_chooser::FileChooser::listen(view).await.map_err(|error|error.to_string())?;
        early.arm();
        let point=evaluate(view,"(()=>{const frame=document.getElementById('cross');const r=frame.getBoundingClientRect();const p=framePickPoints.Cross;return {x:r.x+frame.clientLeft+p.x,y:r.y+frame.clientTop+p.y}})()").await?;
        for kind in ["mousePressed","mouseReleased"] {
            native::protocol_call(view,"Input.dispatchMouseEvent",json!({"type":kind,"x":point["x"],"y":point["y"],"button":"left","buttons":if kind=="mousePressed" {1} else {0},"clickCount":1})).await?;
        }
        let early_choice=early.next(&cancel).await.map_err(|_|"New OOPIF chooser escaped interception before the first observation")?;
        if early_choice.session.is_empty() {return Err("Early chooser did not originate from a real OOPIF".into());}
        drop(early);
        driver.activate_for_agent(view,&cancel).await.map_err(|error|error.to_string())?;
        let mut child_session_proven=false;
        for level in ["Cross","Same"] {
            for kind in ["direct","custom"] {
                let label=if kind=="direct" {format!("{level} attachment")} else {format!("{level} custom picker")};
                let element=super::browser_frame_input::frame_element(&mut driver,view,&label).await?;
                let mut witness=if kind=="custom" {
                    let witness=native::file_chooser::FileChooser::listen(view).await.map_err(|error|error.to_string())?;
                    witness.arm();Some(witness)
                } else {None};
                if let Err(error) = driver.upload(view,element,&files,&cancel).await {
                    let witness = if let Some(witness) = witness.as_mut() {
                        match witness.next(&cancel).await { Ok(choice) => format!("frame={}, session_empty={}, node={}",choice.frame,choice.session.is_empty(),choice.backend_node), Err(error) => format!("no chooser: {error}") }
                    } else { "direct input".into() };
                    return Err(format!("{level} {kind} upload: {error}; witness {witness}"));
                }
                if let Some(witness)=witness.as_mut() {
                    let event=witness.next(&cancel).await.map_err(|error|error.to_string())?;
                    if level=="Cross" {
                        if event.session.is_empty() {return Err("Cross-site chooser did not use an owned child protocol session".into());}
                        child_session_proven=true;
                    }
                }
            }
        }
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
        let evidence=loop {
            let events=evaluate(view,"frameFiles").await?;
            if events.as_array().is_some_and(|events|events.len()==4) {break events;}
            if tokio::time::Instant::now()>=deadline {return Err(format!("Frame file data did not arrive: {events}"));}
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        };
        for level in ["Cross","Same"] {for kind in ["direct","custom"] {
            let event=evidence.as_array().unwrap().iter().find(|event|event["level"]==level && event["source"]==kind).ok_or("Missing frame upload")?;
            if event["name"]!=name || event["text"]!=payload || event["trusted"]!=true || (kind=="custom" && event["clicked"]!=true) {return Err(format!("Incorrect file selection evidence: {event}"));}
        }}
        if !child_session_proven {return Err("Missing OOPIF ownership evidence".into());}
        let parent=super::browser_frame_input::frame_element(&mut driver,view,"Choose in child frame").await?;
        driver.upload(view,parent,&files,&cancel).await.map_err(|error|format!("Parent-to-child chooser: {error}"))?;
        wait_files(view,5).await?;

        // Page-script navigation happens after observation but before the click.
        // The trigger in the parent stays valid, while the chooser's old child
        // world must not grant files to its replacement document.
        let old=super::browser_frame_input::frame_element(&mut driver,view,"Choose in child frame").await?;
        let nonce=evaluate(view,"frameNonces.Same").await?;
        evaluate(view,"document.getElementById('same').src+='?replacement';true").await?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
        loop {
            if evaluate(view,"frameNonces.Same").await?!=nonce {break;}
            if tokio::time::Instant::now()>=deadline {return Err("Child replacement did not load".into());}
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        let mut witness=native::file_chooser::FileChooser::listen(view).await.map_err(|error|error.to_string())?;
        witness.arm();
        if driver.upload(view,old,&files,&cancel).await.is_ok() {return Err("Old child document authority admitted a new chooser".into());}
        witness.next(&cancel).await.map_err(|_|"The stale-document case did not actually open a native chooser")?;
        drop(witness);
        if evaluate(view,"frameFiles.length").await?!=5 {return Err("Stale chooser received file bytes".into());}
        let fresh=super::browser_frame_input::frame_element(&mut driver,view,"Choose in child frame").await?;
        driver.upload(view,fresh,&files,&cancel).await.map_err(|error|format!("Fresh child chooser: {error}"))?;
        let all=wait_files(view,6).await?;
        for event in all.as_array().unwrap().iter().skip(4) {
            if event["source"]!="parent" || event["name"]!=name || event["text"]!=payload || event["clicked"]!=true || event["trusted"]!=true {return Err(format!("Parent delegated upload did not reach its current child: {event}"));}
        }
        Ok(json!({"new_oopif_intercepted_before_observation":true,"same_process_and_oopif":true,"standard_inputs":true,"custom_detached_inputs":true,"parent_button_child_chooser":true,"stale_child_document_rejected":true,"fresh_child_document_accepted":true,"unicode_names_and_bytes":true,"events":all}))
    }.await;
    let settled = driver
        .settle_agent(view)
        .await
        .map_err(|error| error.to_string());
    let unlocked = native::set_user_input_enabled(view, true).await;
    // Destroy fixture documents before releasing file snapshots retained by them.
    native::protocol_call(view, "Page.navigate", json!({"url":"about:blank"})).await?;
    files.close().map_err(|error| error.to_string())?;
    settled?;
    unlocked?;
    result
}

async fn wait_files(view: &View, count: usize) -> Result<Value, String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let files = evaluate(view, "frameFiles").await?;
        if files.as_array().is_some_and(|files| files.len() == count) {
            return Ok(files);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("Expected {count} file deliveries: {files}"));
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

async fn evaluate(view: &View, expression: &str) -> Result<Value, String> {
    let result = native::protocol_call(view, "Runtime.evaluate", json!({"expression":expression,"returnByValue":true,"awaitPromise":true})).await?;
    if result.get("exceptionDetails").is_some() { return Err("Upload fixture evaluation failed".into()); }
    Ok(result["result"]["value"].clone())
}
