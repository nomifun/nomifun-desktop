//! Owned WebView2 target evidence and an explicit controlled debugger probe.
//! Subscription is passive; the opt-in intervention proves the current fixture
//! document and uniquely matching worker before attaching and resuming it.
use serde_json::{Value,json};
use std::{cell::RefCell,collections::BTreeMap,sync::{Arc,Mutex}};
use webview2_com::{CoTaskMemPWSTR,DevToolsProtocolEventReceivedEventHandler,CallDevToolsProtocolMethodCompletedHandler,
    Microsoft::Web::WebView2::Win32::{ICoreWebView2_11,ICoreWebView2DevToolsProtocolEventReceiver,ICoreWebView2DevToolsProtocolEventReceivedEventArgs2}};
use windows::core::{HSTRING,Interface,PCWSTR,PWSTR};

thread_local! {static REGISTRATIONS:RefCell<BTreeMap<uuid::Uuid,(ICoreWebView2DevToolsProtocolEventReceiver,i64)>>=RefCell::new(BTreeMap::new());}

struct Attached {target_id:String,worker:bool,iframe:bool,root_parent:bool,waiting:Option<bool>}
pub(super) struct Witness {view:tauri::Webview,id:uuid::Uuid,events:Arc<Mutex<Vec<Attached>>>}
impl Witness {
    pub(super) async fn install(view:&tauri::Webview)->Result<Self,String> {
        let id=uuid::Uuid::now_v7();let events=Arc::new(Mutex::new(Vec::new()));let captured=events.clone();
        let (tx,rx)=tokio::sync::oneshot::channel();
        view.with_webview(move |platform| {
            let installed=(||->windows::core::Result<()> {
                let core=unsafe {platform.controller().CoreWebView2()?};
                let name=HSTRING::from("Target.attachedToTarget");
                let receiver=unsafe {core.GetDevToolsProtocolEventReceiver(PCWSTR(name.as_ptr()))?};
                let handler=DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_,args| {
                    let Some(args)=args else{return Ok(())};
                    let mut raw=PWSTR::null();let status=unsafe {args.ParameterObjectAsJson(&mut raw)};
                    let data=CoTaskMemPWSTR::from(raw);status?;
                    if raw.is_null(){return Ok(())}
                    let text=unsafe {raw.as_wide()};if text.len()>64*1024{return Ok(())}
                    let parameters:Value=match serde_json::from_str(&String::from_utf16_lossy(text)){Ok(value)=>value,Err(_)=>return Ok(())};
                    drop(data);
                    let args=args.cast::<ICoreWebView2DevToolsProtocolEventReceivedEventArgs2>()?;
                    let mut raw=PWSTR::null();let status=unsafe {args.SessionId(&mut raw)};
                    let allocation=CoTaskMemPWSTR::from(raw);status?;
                    let root_parent=raw.is_null() || unsafe {raw.as_wide()}.is_empty();drop(allocation);
                    let target=&parameters["targetInfo"];
                    let Some(target_id)=target["targetId"].as_str().filter(|id|!id.is_empty()&&id.len()<=256) else{return Ok(())};
                    let mut output=captured.lock().unwrap();
                    if output.len()<64 {output.push(Attached{target_id:target_id.into(),worker:target["type"]=="worker",
                        iframe:target["type"]=="iframe",root_parent,waiting:parameters["waitingForDebugger"].as_bool()});}
                    Ok(())
                }));
                let mut token=0;unsafe {receiver.add_DevToolsProtocolEventReceived(&handler,&mut token)?};
                REGISTRATIONS.with(|entries|entries.borrow_mut().insert(id,(receiver,token)));Ok(())
            })().map_err(|_|"Owned native attached-target witness unavailable".to_owned());
            let _=tx.send(installed);
        }).map_err(|_|"Owned native worker witness view unavailable".to_owned())?;
        let witness=Self{view:view.clone(),id,events};
        rx.await.map_err(|_|"Owned worker witness installation lost".to_owned())??;Ok(witness)
    }
    pub(super) async fn snapshot(&self,worker_url:&str)->Value {
        let targets=super::windows::protocol_call(&self.view,"Target.getTargets",json!({})).await;
        let (Ok(targets),Ok(expected))=(targets,url::Url::parse(worker_url)) else {
            return json!({"owned_target_query_available":false});
        };
        let events=self.events.lock().unwrap();
        let workers:Vec<_>=targets["targetInfos"].as_array().into_iter().flatten()
            .filter(|target|target["type"]=="worker" && target["url"].as_str().and_then(|url|url::Url::parse(url).ok())
                .is_some_and(|url|url.origin()==expected.origin() && url.path()==expected.path()))
            .map(|target|{
                let event=events.iter().rev().find(|event|event.worker && Some(event.target_id.as_str())==target["targetId"].as_str());
                json!({"kind":"controlled_worker","attached":target["attached"].as_bool(),
                    "owned_root_core_event_for_same_native_target":event.is_some(),
                    "root_parent":event.map(|event|event.root_parent),
                    "waiting_for_debugger":event.and_then(|event|event.waiting),
                    "legacy_iframe_only_filter_admits":false})
            }).collect();
        json!({"owned_target_query_available":true,"controlled_worker_targets":workers,
            "owned_attached_event_counts":{"iframe":events.iter().filter(|event|event.iframe).count(),
                "worker_ignored_by_iframe_routes":events.iter().filter(|event|event.worker).count(),
                "other":events.iter().filter(|event|!event.worker&&!event.iframe).count()},
            "autoattach_filter_mutated_by_witness":false})
    }
    /// Diagnostic intervention on exactly one controlled worker, never an
    /// arbitrary target from the model or a shared-profile target by name.
    pub(super) async fn resume_owned_worker(&self,worker_url:&str,page_url:&str,fixture:&super::navigation_fixture::Fixture)->Result<Value,String> {
        let page=super::evaluate(&self.view,"({url:location.href,nonce:fixtureDocumentNonce})").await?;
        if page["url"]!=page_url || !page["nonce"].is_string() {return Err("Controlled worker document ownership proof failed".into());}
        let expected=url::Url::parse(worker_url).map_err(|_|"Controlled worker URL invalid")?;
        let targets=super::windows::protocol_call(&self.view,"Target.getTargets",json!({})).await?;
        let workers:Vec<_>=targets["targetInfos"].as_array().into_iter().flatten().filter(|target|
            target["type"]=="worker" && target["url"].as_str().and_then(|url|url::Url::parse(url).ok())
                .is_some_and(|url|url.origin()==expected.origin()&&url.path()==expected.path())).collect();
        if workers.len()!=1 || workers[0]["attached"]!=false {return Ok(json!({"intervention_attempted":false,"reason":"Unique unattached controlled worker unavailable"}));}
        let id=workers[0]["targetId"].as_str().filter(|id|!id.is_empty()&&id.len()<=256).ok_or("Controlled worker native target missing")?;
        let attached=super::windows::protocol_call(&self.view,"Target.attachToTarget",json!({"targetId":id,"flatten":true})).await?;
        let session=attached["sessionId"].as_str().filter(|id|!id.is_empty()&&id.len()<=256).ok_or("Controlled worker attachment receipt missing")?.to_owned();
        let result=async {
            let same_page=super::evaluate(&self.view,"({url:location.href,nonce:fixtureDocumentNonce})").await?;
            if same_page!=page {return Err("Controlled worker document changed before debugger intervention".to_owned());}
            let attached_witness=self.snapshot(worker_url).await;
            let attach_deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(2);
            let after_attach=loop {
                let state=super::evaluate(&self.view,"({nonce:fixtureDocumentNonce,worker_received:fixtureWorker!==null})").await?;
                if state["nonce"]!=page["nonce"] {return Err("Controlled worker document changed during attach-only observation".into());}
                if state["worker_received"]==true || tokio::time::Instant::now()>=attach_deadline {break state;}
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            };
            let execution_after_attach=fixture.count("/nav/worker-executed");
            run_owned_worker(&self.view,session.clone()).await?;
            let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(2);
            let page_after=loop {
                let state=super::evaluate(&self.view,"({nonce:fixtureDocumentNonce,worker_received:fixtureWorker!==null})").await?;
                if state["nonce"]!=page["nonce"] {return Err("Controlled worker document changed after debugger intervention".into());}
                if state["worker_received"]==true || tokio::time::Instant::now()>=deadline {break state;}
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            };
            Ok(json!({"intervention_attempted":true,"same_owned_document_before_and_after":true,
                "explicit_attached_event":attached_witness,"run_if_waiting_acknowledged":true,
                "attach_only":{"worker_message":after_attach["worker_received"],"executed_http":execution_after_attach,
                    "bounded_observation_ms":2000},
                "worker_message_after_intervention":page_after["worker_received"],"autoattach_filter_changed":false,
                "executed_http_after_run_if_waiting":fixture.count("/nav/worker-executed"),
                "baseline_assertion_replaced":false}))
        }.await;
        let detached=super::windows::protocol_call(&self.view,"Target.detachFromTarget",json!({"sessionId":session})).await;
        match (result,detached) {
            (Ok(mut value),Ok(_))=>{value["owned_diagnostic_session_detached"]=json!(true);Ok(value)}
            (Err(error),Ok(_))=>Err(error),
            (_,Err(_))=>Err("Controlled worker diagnostic session could not detach".into()),
        }
    }
}

async fn run_owned_worker(view:&tauri::Webview,session:String)->Result<(),String> {
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let sender=std::rc::Rc::new(RefCell::new(Some(tx)));let callback_sender=sender.clone();
        let callback=CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |status,response| {
            let result=status.map_err(|_|"Controlled worker debugger command failed".to_owned()).and_then(|_| {
                if response.len()>65536 {return Err("Controlled worker debugger response exceeded limit".into());}
                let parsed:Value=serde_json::from_str(&response).map_err(|_|"Controlled worker debugger response invalid".to_owned())?;
                if parsed.get("error").is_some() {Err("Controlled worker debugger command rejected".into())} else {Ok(())}
            });
            if let Some(tx)=callback_sender.borrow_mut().take(){let _=tx.send(result);}Ok(())
        }));
        let session=HSTRING::from(session);let method=HSTRING::from("Runtime.runIfWaitingForDebugger");let params=HSTRING::from("{}");
        let submitted=unsafe {platform.controller().CoreWebView2().and_then(|core|core.cast::<ICoreWebView2_11>()?
            .CallDevToolsProtocolMethodForSession(PCWSTR(session.as_ptr()),PCWSTR(method.as_ptr()),PCWSTR(params.as_ptr()),&callback))};
        if submitted.is_err(){if let Some(tx)=sender.borrow_mut().take(){let _=tx.send(Err("Controlled worker debugger submission failed".into()));}}
    }).map_err(|_|"Controlled worker view unavailable".to_owned())?;
    rx.await.map_err(|_|"Controlled worker debugger acknowledgement lost".to_owned())?
}
impl Drop for Witness {
    fn drop(&mut self){let id=self.id;let _=self.view.with_webview(move |_|{
        REGISTRATIONS.with(|entries|{if let Some((receiver,token))=entries.borrow_mut().remove(&id){let _=unsafe {receiver.remove_DevToolsProtocolEventReceived(token)};}});
    });}
}
