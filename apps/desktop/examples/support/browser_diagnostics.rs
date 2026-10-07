use super::{evaluate, native_fixture_click, windows};
use nomifun_browser_platform::runtime::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

pub async fn verify(view: &tauri::Webview, url: &str) -> Result<Value, String> {
    let metadata=Arc::new(Mutex::new(BrowserTabSnapshot {
        target:BrowserTabTarget {tab_id:view.label().into(),runtime_generation:1,document_generation:1},
        title:String::new(),url:url.into(),lifecycle:BrowserTabLifecycle::Ready,can_go_back:false,can_go_forward:false,zoom_percent:100,
        blocked_permissions:vec![],permission_requests:vec![],script_dialog:None,diagnostics:Default::default(),
    }));
    windows::diagnostics::install(view,metadata.clone()).await;
    let mut sessions=windows::frames::FrameSessions::connect(view).await?;
    windows::protocol_call(view,"Page.navigate",json!({"url":url})).await?;
    let mut child_count=0;
    for generation in 0..2 {
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
        loop {
            // A planned frame replacement can invalidate a read-only tree or
            // context between callbacks. Retry observation, never the click.
            let readiness=async {
            let trees=sessions.trees().await?;
            child_count=child_count.max(trees.children.len());
            let mut ready=trees.children.len()>=2;
            for (frame,_) in &trees.children {
                let result=sessions.command(frame,"Runtime.evaluate",json!({"expression":format!("document.readyState==='complete' && Number(new URLSearchParams(location.search).get('generation')||0)==={generation}"),"returnByValue":true})).await?;
                ready &= result["result"]["value"]==true;
            }
            Ok::<_,String>(ready && evaluate(view,&format!("document.readyState==='complete' && document.querySelector('#same').contentDocument.readyState==='complete' && Number(new URL(document.querySelector('#same').contentWindow.location.href).searchParams.get('generation')||0)==={generation}")).await?==true)
            }.await;
            if matches!(readiness,Ok(true)) {break;}
            if tokio::time::Instant::now()>=deadline {return Err(format!("Diagnostic fixture generation {generation} did not settle two real iframe sessions: {readiness:?}"));}
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        native_fixture_click(view,"#errors").await?;
        loop {
            let data=metadata.lock().unwrap().diagnostics.clone();
            let messages:Vec<_>=data.entries.iter().map(|entry|entry.message.as_str()).collect();
            let all=messages.iter().any(|message|message.contains(&format!("diag-root-{generation}"))) && ["same","foreign","nested"].iter().all(|scope|
                messages.iter().any(|message|message.contains(&format!("diag-console-{scope}-{generation}"))) &&
                messages.iter().any(|message|message.contains(&format!("diag-exception-{scope}-{generation}"))));
            let network=["same","foreign","nested"].iter().all(|scope|data.entries.iter().any(|entry|entry.kind=="network"&&entry.message=="HTTP 503"&&entry.source_url.ends_with(&format!("/diagnostic-failure/{scope}"))));
            if all && network {
                if data.entries.iter().any(|entry|entry.source_url.contains("not-in-metadata")) {return Err("Diagnostic source URL leaked a query".into());}
                break;
            }
            if tokio::time::Instant::now()>=deadline {return Err(format!("Frame diagnostics incomplete: {}",serde_json::to_string(&data).unwrap()));}
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        for (frame,_) in sessions.trees().await?.children {
            let result=sessions.command(&frame,"Runtime.evaluate",json!({"expression":"window.getterCalls","returnByValue":true})).await?;
            if result["result"]["value"]!=0 {return Err("Frame diagnostics invoked a page getter".into());}
        }
        if generation==0 {
            metadata.lock().unwrap().diagnostics=Default::default();
            native_fixture_click(view,"#replace").await?;
        }
    }
    if evaluate(view,"diagnosticClickTrusted").await?!=true {return Err("Diagnostic fixture click was not trusted".into());}
    Ok(json!({"attached_iframe_sessions":child_count,"same_process_iframe":true,"nested_oopif_console_and_exceptions":true,"network_failure":true,"frame_navigation":true,"no_getter_evaluation":true,"metadata_query_redaction":true}))
}
