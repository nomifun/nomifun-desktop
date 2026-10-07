//! Native user accelerators, routed to the current AgentSession Browser panel.
use nomifun_browser_platform::runtime::{BrowserTabSnapshot, BrowserTabTarget};
use std::{cell::RefCell, collections::HashMap, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}};
use tauri::{Emitter, Manager};
use webview2_com::{AcceleratorKeyPressedEventHandler, Microsoft::Web::WebView2::Win32::*};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all="snake_case")]
enum Action { Address, NewTab, CloseTab, Reload, Back, Forward }
fn action(key:u32, ctrl:bool, alt:bool, shift:bool)->Option<Action> {
    if shift {return None;}
    match (key,ctrl,alt) {
        (0x4c,true,false)=>Some(Action::Address),
        (0x54,true,false)=>Some(Action::NewTab),
        (0x57,true,false)=>Some(Action::CloseTab),
        (0x52,true,false)|(0x74,false,false)=>Some(Action::Reload),
        (0x25,false,true)=>Some(Action::Back),
        (0x27,false,true)=>Some(Action::Forward),
        _=>None,
    }
}
#[derive(Clone, serde::Serialize)]
struct Shortcut { agent_session_id:String, target:BrowserTabTarget, action:Action }
struct Registration { controller:ICoreWebView2Controller, token:i64 }
impl Drop for Registration {
    fn drop(&mut self) { let _=unsafe{self.controller.remove_AcceleratorKeyPressed(self.token)}; }
}
thread_local! {static REGISTERED:RefCell<HashMap<String,Registration>>=RefCell::default();}
pub(super) fn close_view(label:&str) { REGISTERED.with(|all|all.borrow_mut().remove(label)); }

pub(crate) async fn install(view:&tauri::Webview, agent_session_id:String, metadata:Arc<Mutex<BrowserTabSnapshot>>, locked:Arc<AtomicBool>)->Result<(),String> {
    let label=view.label().to_owned();
    let app=view.app_handle().clone();
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result=(|| unsafe {
            if REGISTERED.with(|all|all.borrow().contains_key(&label)) { return Ok(()); }
            let controller=platform.controller();
            let callback=AcceleratorKeyPressedEventHandler::create(Box::new(move |_,args| {
                let args=args.ok_or_else(windows::core::Error::from_win32)?;
                let mut kind=COREWEBVIEW2_KEY_EVENT_KIND::default(); let mut key=0; let mut lparam=0;
                args.KeyEventKind(&mut kind)?; args.VirtualKey(&mut key)?; args.KeyEventLParam(&mut lparam)?;
                if kind!=COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind!=COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {return Ok(());}
                let down=|key|GetKeyState(key) < 0;
                if down(VK_LWIN.0 as i32) || down(VK_RWIN.0 as i32) {return Ok(());}
                let Some(action)=action(key,down(VK_CONTROL.0 as i32),down(VK_MENU.0 as i32),down(VK_SHIFT.0 as i32)) else {return Ok(());};
                // Decide synchronously before any cross-process work. A held key
                // must not create/close a stream of tabs, nor replay after unlock.
                args.SetHandled(true)?;
                if locked.load(Ordering::Acquire) || lparam & (1 << 30) != 0 {return Ok(());}
                let target=metadata.lock().unwrap_or_else(|e|e.into_inner()).target.clone();
                let (app,locked,metadata,agent_session_id)=(app.clone(),locked.clone(),metadata.clone(),agent_session_id.clone());
                tauri::async_runtime::spawn(async move {
                    let dispatch_app=app.clone();
                    let _=app.run_on_main_thread(move || {
                        if locked.load(Ordering::Acquire) || super::native_closed(&target.tab_id) || dispatch_app.get_webview(&target.tab_id).is_none()
                            || metadata.lock().unwrap_or_else(|e|e.into_inner()).target!=target {return;}
                        let Some(main)=dispatch_app.get_webview("main") else {return;};
                        if matches!(action,Action::Address|Action::NewTab) && main.set_focus().is_err() {return;}
                        let _=dispatch_app.emit_to("main","browser-capability-shortcut",Shortcut{agent_session_id,target,action});
                    });
                });
                Ok(())
            }));
            let mut token=0; controller.add_AcceleratorKeyPressed(&callback,&mut token)?;
            REGISTERED.with(|all|all.borrow_mut().insert(label,Registration{controller,token}));
            Ok::<_,windows::core::Error>(())
        })();
        let _=tx.send(result.map_err(|_|"Native browser shortcut registration failed".to_owned()));
    }).map_err(|_|"Native browser is unavailable".to_owned())?;
    rx.await.map_err(|_|"Native shortcut registration result lost".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_browser_keys_without_intercepting_altgr_or_shift_variants() {
        assert_eq!(action(0x4c,true,false,false),Some(Action::Address));
        assert_eq!(action(0x54,true,false,false),Some(Action::NewTab));
        assert_eq!(action(0x57,true,false,false),Some(Action::CloseTab));
        assert_eq!(action(0x52,true,false,false),Some(Action::Reload));
        assert_eq!(action(0x74,false,false,false),Some(Action::Reload));
        assert_eq!(action(0x25,false,true,false),Some(Action::Back));
        assert_eq!(action(0x27,false,true,false),Some(Action::Forward));
        for key in [0x4c,0x54,0x57,0x52] {
            assert_eq!(action(key,true,true,false),None);
            assert_eq!(action(key,true,false,true),None);
            assert_eq!(action(key,false,false,false),None);
        }
    }
}
