//! Real managed UserReady browser, native picker and lifecycle boundaries.
use super::windows::user_file_picker::NativeFilePicker;
use nomifun_browser_platform::{runtime::*, workspace::BrowserResourceService};
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, sync::{Arc, Mutex}};
use tauri::Manager;
use webview2_com::{CoTaskMemPWSTR, DevToolsProtocolEventReceivedEventHandler,
    Microsoft::Web::WebView2::Win32::{ICoreWebView2DevToolsProtocolEventReceiver,
        ICoreWebView2DevToolsProtocolEventReceivedEventArgs2}};
use windows::core::{HSTRING, Interface, PCWSTR, PWSTR};

fn phase(action: &str, name: &str, evidence: Value) {
    eprintln!("BROWSER_USER_FILES_TRACE {}", json!({"action":action,"phase":name,"evidence":evidence}));
}

#[derive(Default)]
struct ChooserFacts {
    events: u8,
    session_present: bool,
    controlled_input_metadata: bool,
    unreadable_event: bool,
}
thread_local! {
    static CHOOSER_TRACES: RefCell<BTreeMap<uuid::Uuid, (ICoreWebView2DevToolsProtocolEventReceiver, i64)>> = RefCell::default();
}
/// Passive fixture witness. It never enables interception, consumes an event,
/// stores native IDs, or changes the production one-shot chooser listener.
struct ChooserTrace {
    view: tauri::Webview,
    id: uuid::Uuid,
    facts: Arc<Mutex<ChooserFacts>>,
}
impl ChooserTrace {
    async fn install(view: &tauri::Webview) -> Result<Self, String> {
        let id=uuid::Uuid::now_v7();
        let facts=Arc::new(Mutex::new(ChooserFacts::default()));
        let observed=facts.clone();
        let (tx,rx)=tokio::sync::oneshot::channel();
        view.with_webview(move |platform| {
            let result=(|| -> windows::core::Result<()> {
                let core=unsafe {platform.controller().CoreWebView2()?};
                let name=HSTRING::from("Page.fileChooserOpened");
                let receiver=unsafe {core.GetDevToolsProtocolEventReceiver(PCWSTR(name.as_ptr()))?};
                let handler=DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_,args| {
                    let evidence=(|| -> Option<(bool,bool)> {
                        let args=args?;
                        let args2=args.cast::<ICoreWebView2DevToolsProtocolEventReceivedEventArgs2>().ok()?;
                        let mut raw=PWSTR::null();
                        let status=unsafe {args2.SessionId(&mut raw)};
                        let allocation=CoTaskMemPWSTR::from(raw);
                        let session=if raw.is_null() {Some(false)} else {
                            let text=unsafe {raw.as_wide()};
                            (text.len()<=256).then_some(!text.is_empty())
                        };
                        drop(allocation);status.ok()?;
                        let mut raw=PWSTR::null();
                        let status=unsafe {args.ParameterObjectAsJson(&mut raw)};
                        let allocation=CoTaskMemPWSTR::from(raw);
                        let value=if raw.is_null() {None} else {
                            let text=unsafe {raw.as_wide()};
                            if text.len()>8192 {None} else {serde_json::from_str::<Value>(&String::from_utf16_lossy(text)).ok()}
                        };
                        drop(allocation);status.ok()?;
                        let value=value?;
                        let valid=value["frameId"].as_str().is_some_and(|frame|!frame.is_empty()&&frame.len()<=256)
                            && value["backendNodeId"].as_i64().is_some_and(|node|node>0)
                            && matches!(value["mode"].as_str(),Some("selectSingle"|"selectMultiple"));
                        Some((session?,valid))
                    })();
                    let mut facts=observed.lock().unwrap_or_else(|error|error.into_inner());
                    facts.events=facts.events.saturating_add(1);
                    if let Some((session,valid))=evidence {
                        facts.session_present|=session;
                        facts.controlled_input_metadata|=valid;
                    } else {facts.unreadable_event=true;}
                    Ok(())
                }));
                let mut token=0;
                unsafe {receiver.add_DevToolsProtocolEventReceived(&handler,&mut token)?;}
                CHOOSER_TRACES.with(|entries|entries.borrow_mut().insert(id,(receiver,token)));
                Ok(())
            })().map_err(|_|"Controlled chooser event witness registration failed".to_owned());
            let _=tx.send(result);
        }).map_err(|_|"Controlled chooser event witness UI dispatch failed")?;
        let witness=Self {view:view.clone(),id,facts};
        rx.await.map_err(|_|"Controlled chooser event witness registration callback lost")??;
        Ok(witness)
    }
    fn snapshot(&self) -> Value {
        let facts=self.facts.lock().unwrap_or_else(|error|error.into_inner());
        json!({"controlled_native_event_seen":facts.events>0,"event_count":facts.events,
            "nonempty_native_session_seen":facts.session_present,
            "valid_input_target_metadata_seen":facts.controlled_input_metadata,
            "unreadable_event_seen":facts.unreadable_event})
    }
}
impl Drop for ChooserTrace {
    fn drop(&mut self) {
        let id=self.id;
        let _=self.view.with_webview(move |_| {
            CHOOSER_TRACES.with(|entries| {
                if let Some((receiver,token))=entries.borrow_mut().remove(&id) {
                    let _=unsafe {receiver.remove_DevToolsProtocolEventReceived(token)};
                }
            });
        });
    }
}

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
        BrowserResourceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let authority = super::browser_resource_fixture::authority(
        "user-files-fixture",
        "user-files-fixture",
        "native-user-files",
    );
    let key = authority.key();
    let workspace = service
        .ensure(
            authority,
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
            let chooser_trace=if action=="frame_navigate" {
                Some(ChooserTrace::install(&view).await.map_err(|error|format!("{action}/listener_ready: {error}"))?)
            } else {None};
            if action=="frame_navigate" {
                let trace=chooser_trace.as_ref().unwrap();
                let mut witness=super::windows::file_chooser::FileChooser::listen(&view).await.map_err(|error|format!("{action}/listener_ready: {error}"))?;
                witness.arm();
                phase(action,"listener_ready",trace.snapshot());
                cross_click(&view).await.map_err(|error|format!("{action}/input_complete: {error}"))?;
                phase(action,"input_complete",trace.snapshot());
                phase(action,"chooser_wait",trace.snapshot());
                let choice=match witness.next(&Default::default()).await {
                    Ok(choice)=>choice,
                    Err(error)=>{
                        let child_click=super::evaluate(&view,"({click:window.crossClick,child:window.crossInputs,root:window.rootInputs,point:window.crossPoint,focused:document.hasFocus(),visible:document.visibilityState,ready:window.crossReady,frame:document.getElementById('cross').getBoundingClientRect().toJSON()})").await?;
                        let native_visible=super::native_visible(&view).await?;
                        let native_bounds=super::native_bounds(&view).await?;
                        return Err(format!("{action}/chooser_wait: {error}; controlled_chooser={}; controlled_child_click={child_click}; native_visible={native_visible}; native_bounds={native_bounds:?}",trace.snapshot()));
                    },
                };
                // Protocol events and window.postMessage use independent
                // delivery queues. Await only the passive click witness.
                let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
                let child_click=loop {
                    let child_click=super::evaluate(&view,"window.crossClick").await?;
                    if !child_click.is_null() {break child_click;}
                    if tokio::time::Instant::now()>=deadline {return Err(format!("{action}/chooser_received: Native child click witness was not delivered"));}
                    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                };
                if child_click["trusted"]!=true || child_click["activation"]!=true {
                    return Err(format!("{action}/chooser_received: Native child click lacked trusted user activation: {child_click}"));
                }
                phase(action,"chooser_received",json!({"nonempty_choice_session":!choice.session.is_empty(),
                    "choice_has_frame":!choice.frame.is_empty(),"choice_has_input_target":choice.backend_node>0,
                    "controlled_chooser":trace.snapshot(),"controlled_child_click":child_click}));
                if choice.session.is_empty() {return Err(format!("{action}/chooser_received: Managed user chooser did not use an OOPIF session"));}
            } else {
                super::native_fixture_click(&view,"#file").await.map_err(|error|format!("{action}/input_complete: {error}"))?;
                phase(action,"input_complete",json!({"root_input_dispatch_completed":true}));
            }
            let picker=picker(&view).await.map_err(|error|format!("{action}/native_picker_open: {error}"))?;
            phase(action,"native_picker_open",json!({"native_picker_opened":true}));
            let receipt=picker.exit_receipt().await.map_err(|error|format!("{action}/receipt_exit: exit receipt unavailable: {error}"))?;
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
                    super::evaluate(&view,"crossReady=false;document.getElementById('cross').src+='?target-replacement';true").await.map_err(|error|format!("{action}/frame_replaced: dispatch failed: {error}"))?;
                    let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(3);
                    while super::evaluate(&view,"crossReady").await.map_err(|error|format!("{action}/frame_replaced: readiness read failed: {error}"))?!=true {
                        if tokio::time::Instant::now()>=deadline {return Err(format!("{action}/frame_replaced: Target iframe did not navigate"));}
                        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
                    }
                    phase(action,"frame_replaced",json!({"target_frame_ready_after_replacement":true,
                        "controlled_chooser":chooser_trace.as_ref().unwrap().snapshot()}));
                },
                "close"=>{ service.close(&key).await.map_err(|e|e.to_string())?; },
                _=>unreachable!(),
            }
            tokio::time::timeout(std::time::Duration::from_secs(5),receipt.wait()).await.map_err(|_|format!("{action}/receipt_exit: did not close the picker process"))?.map_err(|error|format!("{action}/receipt_exit: {error}"))?;
            phase(action,"receipt_exit",json!({"native_picker_exit_acknowledged":true}));
            if picker.finished().await.map_err(|error|format!("{action}/receipt_exit: picker result failed: {error}"))?.is_some() { return Err(format!("{action}/receipt_exit: returned selected files")); }
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
    // its click geometry) remains current after a document replacement/reflow.
    // Observe this exact request's current child geometry before input; never
    // retry a click after it may have opened a chooser.
    let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
    while super::evaluate(view,"window.crossReady===true").await?!=true {
        if tokio::time::Instant::now()>=deadline {return Err("OOPIF file picker document was not ready before observation".into());}
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    }
    let request=uuid::Uuid::now_v7().to_string();
    super::evaluate(view,&format!("(()=>{{window.crossPointRequest={request:?};window.crossPointReceipt=null;const frame=document.getElementById('cross');frame.contentWindow.postMessage({{kind:'picker-point-request',request:window.crossPointRequest}},new URL(frame.src).origin);return true}})()")).await?;
    let point=loop {
        let point=super::evaluate(view,"(()=>{const receipt=window.crossPointReceipt;if(!window.crossReady||!receipt||receipt.request!==window.crossPointRequest)return null;const f=document.getElementById('cross');const r=f.getBoundingClientRect();const x=r.x+f.clientLeft+receipt.pickPoint.x,y=r.y+f.clientTop+receipt.pickPoint.y;return Number.isFinite(x)&&Number.isFinite(y)&&document.elementFromPoint(x,y)===f?{x,y}:null})()").await?;
        if !point.is_null() {break point;}
        if tokio::time::Instant::now()>=deadline {return Err("OOPIF file picker button was not ready and hittable before input".into());}
        tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    };
    // Mirror the native click path: move to the current hit target before the
    // single press/release. Chromium routes OOPIF input through a separate
    // widget; the previous root-file click is not a pointer move into it.
    for event in ["mouseMoved", "mousePressed", "mouseReleased"] {
        super::windows::protocol_call(view,"Input.dispatchMouseEvent",json!({"type":event,"x":point["x"],"y":point["y"],"button":if event=="mouseMoved"{"none"}else{"left"},"buttons":if event=="mousePressed"{1}else{0},"clickCount":1})).await?;
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
