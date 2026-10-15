//! Passive native NewWindowRequested evidence on the exact fixture opener.
//! This never sets Handled, obtains a deferral or changes popup admission.
use serde_json::{Value,json};
use std::{cell::RefCell,collections::BTreeMap,sync::{Arc,Mutex}};
use tauri::Manager;
use webview2_com::{CoTaskMemPWSTR,NewWindowRequestedEventHandler,
    Microsoft::Web::WebView2::Win32::ICoreWebView2};
use windows::core::{BOOL,PWSTR};

thread_local! {static REGISTRATIONS:RefCell<BTreeMap<uuid::Uuid,(ICoreWebView2,i64)>>=RefCell::default();}

fn remove_registration(id:uuid::Uuid)->Result<(),String> {
    let registration=REGISTRATIONS.with(|entries|entries.borrow().get(&id).cloned());
    if let Some((core,token))=registration {
        // Keep the exact native owner until removal is acknowledged. COM work
        // stays outside a registry borrow, including synchronous reentrancy.
        unsafe{core.remove_NewWindowRequested(token)}.map_err(|_|"Native popup witness removal failed".to_owned())?;
        REGISTRATIONS.with(|entries|entries.borrow_mut().remove(&id));
    }
    Ok(())
}

#[derive(Clone)]
struct Event {user_initiated:bool,handled_by_host:bool,path:String}
pub(super) struct Witness {view:tauri::Webview,id:uuid::Uuid,events:Arc<Mutex<Vec<Event>>>}
impl Witness {
    pub(super) async fn install(view:&tauri::Webview)->Result<Self,String> {
        let id=uuid::Uuid::now_v7();let events=Arc::new(Mutex::new(Vec::new()));let captured=events.clone();
        let (tx,rx)=tokio::sync::oneshot::channel();
        view.with_webview(move|platform|{
            let result=(||->windows::core::Result<()> {
                let core=unsafe{platform.controller().CoreWebView2()?};
                let handler=NewWindowRequestedEventHandler::create(Box::new(move|_,args|{
                    let Some(args)=args else{return Ok(())};
                    let mut initiated=BOOL::default();let mut handled=BOOL::default();let mut raw=PWSTR::null();
                    unsafe{args.IsUserInitiated(&mut initiated)?;args.Handled(&mut handled)?;args.Uri(&mut raw)?;}
                    let allocation=CoTaskMemPWSTR::from(raw);
                    let path=if raw.is_null(){None}else{
                        let units=unsafe{raw.as_wide()};
                        (units.len()<=8192).then(||String::from_utf16_lossy(units))
                            .and_then(|uri|url::Url::parse(&uri).ok()).map(|uri|uri.path().to_owned())
                    };
                    drop(allocation);
                    let path=path.filter(|path|matches!(path.as_str(),"/nav/popup"|"/nav/popup-blank"|"blank"))
                        .unwrap_or_else(||"uncontrolled_or_unreadable".into());
                    let mut output=captured.lock().unwrap();
                    if output.len()<32 {output.push(Event{user_initiated:initiated.as_bool(),handled_by_host:handled.as_bool(),path});}
                    Ok(())
                }));
                let mut token=0;unsafe{core.add_NewWindowRequested(&handler,&mut token)?};
                REGISTRATIONS.with(|entries|entries.borrow_mut().insert(id,(core,token)));Ok(())
            })().map_err(|_|"Native popup request witness installation failed".to_owned());
            let _=tx.send(result);
        }).map_err(|_|"Native popup request witness view unavailable".to_owned())?;
        // Own removal before awaiting the installation receipt, so caller
        // cancellation queues cleanup behind the UI-thread installation.
        let witness=Self{view:view.clone(),id,events};
        rx.await.map_err(|_|"Native popup request witness callback lost".to_owned())??;
        Ok(witness)
    }
    pub(super) fn count(&self)->usize {self.events.lock().unwrap().len()}
    pub(super) fn since(&self,start:usize)->Value {
        let events=self.events.lock().unwrap();
        json!({"passive_native_event_observer":true,"native_events":events.iter().skip(start)
            .map(|event|json!({"path":event.path,"is_user_initiated":event.user_initiated,"host_handled":event.handled_by_host})).collect::<Vec<_>>()})
    }
    pub(super) async fn close(self)->Result<(),String> {
        let id=self.id;let (tx,rx)=tokio::sync::oneshot::channel();
        self.view.app_handle().run_on_main_thread(move||{
            let _=tx.send(remove_registration(id));
        }).map_err(|_|"Native popup witness removal dispatch failed".to_owned())?;
        rx.await.map_err(|_|"Native popup witness removal callback lost".to_owned())?
    }
}
impl Drop for Witness {
    fn drop(&mut self){let id=self.id;let _=self.view.app_handle().run_on_main_thread(move||{
        let _=remove_registration(id);
    });}
}
