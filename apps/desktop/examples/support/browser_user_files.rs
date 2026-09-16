//! Real managed UserReady browser, native picker and lifecycle boundaries.
use super::windows::user_file_picker::NativeFilePicker;
use nomifun_browser_platform::{runtime::*, workspace::BrowserWorkspaceService};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::Manager;

async fn picker(view: &tauri::Webview) -> Result<NativeFilePicker, String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        if let Some(picker) = super::windows::user_file_chooser::active_picker(view).await? {
            picker.opened().await?;
            return Ok(picker);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("User HTML chooser did not open an application native picker".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    }
}
#[derive(Clone, Copy)]
pub(super) enum Check {
    Lifecycle,
    Selection,
    Filter,
}
pub(super) async fn verify(
    app: &tauri::AppHandle,
    url: &str,
    check: Check,
) -> Result<Value, String> {
    let service =
        BrowserWorkspaceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let key = BrowserWorkspaceKey {
        user_id: "user-files-fixture".into(),
        conversation_id: "user-files-fixture".into(),
    };
    let workspace = service
        .ensure(
            key.clone(),
            "native-user-files".into(),
            BrowserProfile::Ephemeral,
        )
        .await
        .map_err(|e| e.to_string())?;
    let result=async {
        workspace.user_command(BrowserTabCommand::Create {url:url.into()}).await.map_err(|e|format!("Create user file browser: {e}"))?;
        let target=super::wait_workspace_page(&workspace,url).await?;
        let view=app.get_webview(&target.tab_id).ok_or("Missing user file view")?;
        let bounds=BrowserSurfaceBounds {x:20.,y:60.,width:1000.,height:600.};
        workspace.set_surface(bounds,true,Default::default()).await.map_err(|e|format!("Show user file browser: {e}"))?;
        if matches!(check,Check::Selection) {return select_files(&view).await;}
        if matches!(check,Check::Filter) {return inspect_filter(&view).await;}
        for action in ["hide","navigate","frame_navigate","agent","close"] {
            eprintln!("BROWSER_USER_FILES_PHASE {action}");
            if action=="frame_navigate" {
                let mut witness=super::windows::file_chooser::FileChooser::listen(&view).await.map_err(|e|e.to_string())?;
                witness.arm();
                cross_click(&view).await?;
                let choice=witness.next(&Default::default()).await.map_err(|e|e.to_string())?;
                if choice.session.is_empty() {return Err("Managed user chooser did not use an OOPIF session".into());}
            } else {super::native_fixture_click(&view,"#file").await?;}
            let picker=picker(&view).await?;
            let receipt=picker.exit_receipt().await?;
            if action=="hide" {
                super::evaluate(&view,"crossReady=false;document.getElementById('cross').src+='?unrelated';true").await?;
                let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
                while super::evaluate(&view,"crossReady").await?!=true {
                    if tokio::time::Instant::now()>=deadline {return Err("Unrelated iframe did not navigate".into());}
                    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                }
                if tokio::time::timeout(std::time::Duration::from_millis(100),picker.finished()).await.is_ok() {return Err("Unrelated iframe navigation cancelled the root picker".into());}
            }
            match action {
                "hide"=>workspace.set_surface(bounds,false,Default::default()).await.map_err(|e|e.to_string())?,
                "navigate"=>{
                    let target=workspace.snapshot().await.map_err(|e|e.to_string())?.runtime.ok_or("Missing file runtime")?.tabs[0].target.clone();
                    let next=format!("{url}?replacement");
                    workspace.user_command(BrowserTabCommand::Navigate {target,url:next.clone()}).await.map_err(|e|e.to_string())?;
                    super::wait_workspace_page(&workspace,&next).await?;
                },
                "agent"=>{
                    let run=workspace.begin_run().await.map_err(|e|e.to_string())?;
                    tokio::time::timeout(std::time::Duration::from_millis(100),receipt.clone().wait()).await.map_err(|_|"Agent admitted before picker exit")?.map_err(|e|e.to_string())?;
                    workspace.finish_run(&run).await.map_err(|e|e.to_string())?;
                },
                "frame_navigate"=>{
                    super::evaluate(&view,"crossReady=false;document.getElementById('cross').src+='?target-replacement';true").await?;
                    let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
                    while super::evaluate(&view,"crossReady").await?!=true {
                        if tokio::time::Instant::now()>=deadline {return Err("Target iframe did not navigate".into());}
                        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                    }
                },
                "close"=>{ service.close(&key).await.map_err(|e|e.to_string())?; },
                _=>unreachable!(),
            }
            tokio::time::timeout(std::time::Duration::from_secs(5),receipt.wait()).await.map_err(|_|format!("{action} did not close the picker process"))?.map_err(|e|e.to_string())?;
            if picker.finished().await?.is_some() { return Err(format!("{action} returned selected files")); }
            if action=="hide" {workspace.set_surface(bounds,true,Default::default()).await.map_err(|e|e.to_string())?;}
            if action!="close" && super::evaluate(&view,"userFiles.length").await?!=0 {return Err("Cancelled user picker delivered file data".into());}
        }
        Ok(json!({"native_user_picker":true,"unrelated_iframe_preserves_picker":true,"oopif_document_change_cancels":true,"hide_cancels":true,"navigation_cancels":true,"agent_admission_waits_for_exit":true,"close_waits_for_exit":true,"no_cancelled_files_delivered":true}))
    }.await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match service.close(&key).await {
            Ok(()) => break,
            Err(error) if tokio::time::Instant::now() >= deadline => {
                return Err(format!("User file fixture cleanup: {error}"));
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(25)).await,
        }
    }
    result
}

async fn select_files(view: &tauri::Webview) -> Result<Value, String> {
    let root = tempfile::Builder::new()
        .prefix("nomifun-user-file-data-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    let names = ["用户 选择.txt", "second.txt"];
    let text = "Native user upload fixture 中文";
    for name in names {
        std::fs::write(root.path().join(name), text).map_err(|e| e.to_string())?;
    }
    let mut received = vec![];
    for (index, kind) in ["root", "cross"].into_iter().enumerate() {
        if kind == "root" {
            super::native_fixture_click(view, "#file").await?;
        } else {
            cross_click(view).await?;
        }
        let picker = picker(view).await?;
        eprintln!(
            "BROWSER_USER_FILE_SELECTION_READY {}",
            json!({"kind":kind,"files":if kind=="root" {names.to_vec()}else{vec![names[0]]},"directory":root.path()})
        );
        let paths = picker
            .finished()
            .await?
            .ok_or("Native user selection was cancelled")?;
        if paths.len() != if kind == "root" { 2 } else { 1 } {
            return Err("Wrong native user selection count".into());
        }
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let record = loop {
            let records = super::evaluate(view, "userFiles").await?;
            if let Some(record) = records.get(index) {
                break record.clone();
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(format!("Native {kind} selection did not reach its page"));
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        };
        if record["trusted"] != true {
            return Err("User file change was not browser-generated".into());
        }
        if kind == "root" {
            let files = record["files"].as_array().ok_or("No root files")?;
            if files.len() != 2
                || names.iter().any(|name| {
                    !files
                        .iter()
                        .any(|file| file["name"] == *name && file["text"] == text)
                })
            {
                return Err("Incorrect root user file bytes".into());
            }
        } else if record["name"] != names[0] || record["text"] != text || record["clicked"] != true
        {
            return Err("Incorrect OOPIF user file bytes or native click".into());
        }
        received.push(record);
    }
    super::windows::protocol_call(view, "Page.navigate", json!({"url":"about:blank"})).await?;
    root.close().map_err(|e| e.to_string())?;
    Ok(
        json!({"root_multiple_files":true,"oopif_temporary_input":true,"unicode_and_bytes":true,"native_change_events":true,"files":received}),
    )
}

async fn cross_click(view: &tauri::Webview) -> Result<(), String> {
    // Root load completion does not prove that the child's load message (and
    // its click geometry) has reached the parent after a document replacement.
    // Wait before input; never retry a click after it may have opened a chooser.
    let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
    let point=loop {
        let point=super::evaluate(view,"(()=>{if(!window.crossReady||!window.crossPoint)return null;const f=document.getElementById('cross');const r=f.getBoundingClientRect();const x=r.x+f.clientLeft+crossPoint.x,y=r.y+f.clientTop+crossPoint.y;return Number.isFinite(x)&&Number.isFinite(y)&&document.elementFromPoint(x,y)===f?{x,y}:null})()").await?;
        if !point.is_null() {break point;}
        if tokio::time::Instant::now()>=deadline {return Err("OOPIF file picker button was not ready and hittable before input".into());}
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    };
    for event in ["mousePressed", "mouseReleased"] {
        super::windows::protocol_call(view,"Input.dispatchMouseEvent",json!({"type":event,"x":point["x"],"y":point["y"],"button":"left","buttons":if event=="mousePressed"{1}else{0},"clickCount":1})).await?;
    }
    Ok(())
}

async fn inspect_filter(view: &tauri::Webview) -> Result<Value, String> {
    let root = tempfile::Builder::new()
        .prefix("nomifun-file-filter-")
        .tempdir()
        .map_err(|e| e.to_string())?;
    for name in ["accepted.txt", "accepted.csv", "other.png"] {
        std::fs::write(root.path().join(name), "Local filter fixture")
            .map_err(|e| e.to_string())?;
    }
    super::evaluate(
        view,
        "document.getElementById('file').accept='.txt,text/csv';true",
    )
    .await?;
    super::native_fixture_click(view, "#file").await?;
    let picker = picker(view).await?;
    eprintln!(
        "BROWSER_USER_FILE_FILTER_READY {}",
        json!({"directory":root.path(),"expected_visible":["accepted.txt","accepted.csv"],"expected_hidden":"other.png"})
    );
    if picker.finished().await?.is_some() {
        return Err("Filter inspection must cancel without choosing files".into());
    }
    picker.close().await?;
    if super::evaluate(view, "userFiles.length").await? != 0 {
        return Err("Filter inspection delivered files".into());
    }
    root.close().map_err(|e| e.to_string())?;
    Ok(
        json!({"native_filter_dialog_cancelled":true,"no_files_selected":true,"fixture_cleanup":true}),
    )
}
