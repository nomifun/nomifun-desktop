//! Exact native browser-process observer for the disposable Profile fixture.
//! Only equality and exit booleans leave this module; COM stays on the UI thread.
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, sync::{Arc, Mutex}};
use webview2_com::{BrowserProcessExitedEventHandler, GetCookiesCompletedHandler, CoTaskMemPWSTR, Microsoft::Web::WebView2::Win32::{
    ICoreWebView2, ICoreWebView2Environment5, ICoreWebView2_2, ICoreWebView2_13, ICoreWebView2Profile, ICoreWebView2CookieManager, COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND,
    COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND_NORMAL,
}};
use ::windows::core::{BOOL,Interface,PCWSTR,PWSTR};

pub(super) async fn cookie_manager_witness(view:&tauri::Webview)->Result<Value,String> {
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let manager=(||->::windows::core::Result<ICoreWebView2CookieManager> {
            let core=unsafe{platform.controller().CoreWebView2()?};
            unsafe{core.cast::<ICoreWebView2_2>()?.CookieManager()}
        })();
        match manager {Ok(manager)=>dispatch_cookie_inventory(&manager,tx),Err(_)=>{let _=tx.send(Err("Controlled SDK Cookie manager unavailable".into()));}}
    }).map_err(|_|"Controlled SDK Cookie manager UI dispatch unavailable")?;
    rx.await.map_err(|_|"Controlled SDK Cookie manager callback lost")?
}

pub(super) fn dispatch_cookie_inventory(manager:&ICoreWebView2CookieManager,tx:tokio::sync::oneshot::Sender<Result<Value,String>>) {
        let sender=std::rc::Rc::new(RefCell::new(Some(tx)));
        let completed=sender.clone();
        let result=(||->::windows::core::Result<()> {
            let handler=GetCookiesCompletedHandler::create(Box::new(move |error,cookies| {
                let result=(||->::windows::core::Result<Value> {
                    error?;
                    let Some(cookies)=cookies else{return Ok(json!({"sdk_cookie_present":false}));};
                    let mut count=0;
                    unsafe{cookies.Count(&mut count)?;}
                    if count>32 {return Err(::windows::core::Error::from_hresult(::windows::Win32::Foundation::E_UNEXPECTED));}
                    for index in 0..count {
                        let cookie=unsafe{cookies.GetValueAtIndex(index)?};
                        let mut raw=PWSTR::null();
                        let received=unsafe{cookie.Name(&mut raw)};
                        let allocation=CoTaskMemPWSTR::from(raw);
                        let name=if raw.is_null(){None}else{let text=unsafe{raw.as_wide()};(text.len()<=128).then(||String::from_utf16_lossy(text))};
                        drop(allocation);received?;
                        if name.as_deref()!=Some("nomi_profile_probe") {continue;}
                        let mut session=BOOL::default();let mut http_only=BOOL::default();let mut expires=0.0;
                        unsafe{cookie.IsSession(&mut session)?;cookie.IsHttpOnly(&mut http_only)?;cookie.Expires(&mut expires)?;}
                        let future=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok().map(|now|expires>now.as_secs_f64());
                        return Ok(json!({"sdk_cookie_present":true,"sdk_cookie_session":session.as_bool(),"sdk_cookie_http_only":http_only.as_bool(),"sdk_cookie_expiry_is_future":future}));
                    }
                    Ok(json!({"sdk_cookie_present":false}))
                })().map_err(|_|"Controlled SDK Cookie manager inventory unavailable".to_owned());
                if let Some(sender)=completed.borrow_mut().take(){let _=sender.send(result);}
                Ok(())
            }));
            unsafe{manager.GetCookies(PCWSTR::null(),&handler)?;}
            Ok(())
        })();
        if result.is_err(){if let Some(sender)=sender.borrow_mut().take(){let _=sender.send(Err("Controlled SDK Cookie manager dispatch unavailable".to_owned()));}}
}

struct CookieManagerRegistration { _profile:ICoreWebView2Profile,manager:ICoreWebView2CookieManager }
thread_local! {static COOKIE_MANAGERS:RefCell<BTreeMap<uuid::Uuid,CookieManagerRegistration>>=RefCell::default();}

/// Controlled COM-lifetime intervention; never an application Profile keeper.
pub(super) struct CookieManagerIntervention {id:uuid::Uuid}
impl CookieManagerIntervention {
    pub async fn install(view:&tauri::Webview)->Result<Self,String> {
        let id=uuid::Uuid::now_v7();let (tx,rx)=tokio::sync::oneshot::channel();
        view.with_webview(move|platform| {
            let result=(||->::windows::core::Result<()> {
                let core=unsafe{platform.controller().CoreWebView2()?};
                let profile=unsafe{core.cast::<ICoreWebView2_13>()?.Profile()?};
                let manager=unsafe{core.cast::<ICoreWebView2_2>()?.CookieManager()?};
                COOKIE_MANAGERS.with(|entries|entries.borrow_mut().insert(id,CookieManagerRegistration{_profile:profile,manager}));
                Ok(())
            })().map_err(|_|"Controlled retained Profile/CookieManager unavailable".to_owned());
            let _=tx.send(result);
        }).map_err(|_|"Controlled retained Profile UI dispatch unavailable")?;
        rx.await.map_err(|_|"Controlled retained Profile callback lost")??;
        Ok(Self{id})
    }
    pub async fn snapshot(&self,app:&tauri::AppHandle)->Value {
        let id=self.id;let (tx,rx)=tokio::sync::oneshot::channel();
        let dispatched=app.run_on_main_thread(move|| {
            let manager=COOKIE_MANAGERS.with(|entries|entries.borrow().get(&id).map(|entry|entry.manager.clone()));
            if let Some(manager)=manager {dispatch_cookie_inventory(&manager,tx);}else{let _=tx.send(Err("Controlled retained CookieManager registration absent".into()));}
        });
        if dispatched.is_err(){return json!({"sdk_inventory_available":false});}
        match tokio::time::timeout(std::time::Duration::from_secs(5),rx).await {
            Ok(Ok(Ok(mut value)))=>{value["sdk_inventory_available"]=json!(true);value},
            Ok(_)=>json!({"sdk_inventory_available":false,"known_failure":"native_inventory_unavailable"}),
            Err(_)=>json!({"sdk_inventory_available":false,"known_failure":"native_callback_deadline"}),
        }
    }
    pub async fn close(self,app:&tauri::AppHandle)->Result<(),String> {
        let id=self.id;let (tx,rx)=tokio::sync::oneshot::channel();
        app.run_on_main_thread(move||{COOKIE_MANAGERS.with(|entries|entries.borrow_mut().remove(&id));let _=tx.send(());})
            .map_err(|_|"Controlled retained Profile cleanup dispatch unavailable")?;
        rx.await.map_err(|_|"Controlled retained Profile cleanup callback lost".to_owned())
    }
}

pub(super) struct Discriminator { pid:u32, name:String, profile_path:String }
impl Discriminator {
    pub fn snapshot(&self)->Value {json!({"native_pid_observed":self.pid!=0,"native_profile_name_is_default":self.name=="Default","native_profile_path_observed":!self.profile_path.is_empty()})}
    pub fn compare(&self,other:&Self)->Value {json!({"old_pid_equals_reopened_pid":(self.pid!=0&&other.pid!=0).then_some(self.pid==other.pid),"native_profile_names_equal":self.name==other.name,"native_profile_paths_equal":(!self.profile_path.is_empty()&&!other.profile_path.is_empty()).then_some(self.profile_path==other.profile_path),"original_profile_name_is_default":self.name=="Default","reopened_profile_name_is_default":other.name=="Default"})}
    pub fn path_is_within(&self, intended_udf:&std::path::Path)->Option<bool> {
        let intended=dunce::canonicalize(intended_udf).ok()?;
        let actual=dunce::canonicalize(&self.profile_path).ok()?;
        Some(actual.starts_with(intended))
    }
    pub fn cookie_database_exists_within(&self,intended_udf:&std::path::Path)->Option<bool> {
        if self.path_is_within(intended_udf)!=Some(true){return None;}
        let profile=std::path::Path::new(&self.profile_path);
        Some(profile.join("Network").join("Cookies").is_file()||profile.join("Cookies").is_file())
    }
}
pub(super) async fn discriminate(view:&tauri::Webview)->Result<Discriminator,String> {
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result=unsafe{platform.controller().CoreWebView2()}.map_err(|_|"Profile native discriminator core unavailable".to_owned()).and_then(|core|discriminate_core(&core));
        let _=tx.send(result);
    }).map_err(|_|"Profile native discriminator dispatch unavailable")?;
    rx.await.map_err(|_|"Profile native discriminator dispatch lost")?
}
pub(super) fn discriminate_core(core:&ICoreWebView2)->Result<Discriminator,String> {
            let mut pid=0;
            unsafe{core.BrowserProcessId(&mut pid)}.map_err(|_|"Profile native discriminator process unavailable")?;
            let profile=unsafe{core.cast::<ICoreWebView2_13>().map_err(|_|"Profile native discriminator version unavailable")?.Profile()}.map_err(|_|"Profile native discriminator profile unavailable")?;
            let mut raw=PWSTR::null();
            let received=unsafe{profile.ProfileName(&mut raw)};
            let allocation=CoTaskMemPWSTR::from(raw);
            let name=if raw.is_null(){None}else{let text=unsafe{raw.as_wide()};(text.len()<=128).then(||String::from_utf16_lossy(text))};
            drop(allocation);
            received.map_err(|_|"Profile native discriminator name unavailable")?;
            let mut raw=PWSTR::null();
            let received=unsafe{profile.ProfilePath(&mut raw)};
            let allocation=CoTaskMemPWSTR::from(raw);
            let path=if raw.is_null(){None}else{let text=unsafe{raw.as_wide()};(text.len()<=32768).then(||String::from_utf16_lossy(text))};
            drop(allocation);
            received.map_err(|_|"Profile native discriminator path unavailable")?;
            Ok(Discriminator{pid,name:name.ok_or("Profile native discriminator name invalid")?,profile_path:path.ok_or("Profile native discriminator path invalid")?})
}

#[derive(Default)]
struct State { original_pid: u32, exited: bool, normal: Option<bool> }
struct Registration { environment: ICoreWebView2Environment5, token: i64 }
thread_local! { static REGISTRATIONS: RefCell<BTreeMap<uuid::Uuid, Registration>> = RefCell::default(); }

pub(super) struct Observer { id: uuid::Uuid, state: Arc<Mutex<State>> }
impl Observer {
    pub async fn install(view: &tauri::Webview) -> Result<Self, String> {
        let id = uuid::Uuid::now_v7();
        let state = Arc::new(Mutex::new(State::default()));
        let native_state = state.clone();
        let (tx,rx) = tokio::sync::oneshot::channel();
        view.with_webview(move |platform| {
            let result = (|| -> ::windows::core::Result<()> {
                let core = unsafe { platform.controller().CoreWebView2()? };
                let mut pid = 0;
                unsafe { core.BrowserProcessId(&mut pid)?; }
                native_state.lock().unwrap().original_pid = pid;
                let environment = platform.environment().cast::<ICoreWebView2Environment5>()?;
                let events = native_state.clone();
                let handler = BrowserProcessExitedEventHandler::create(Box::new(move |_,args| {
                    let Some(args) = args else { return Ok(()); };
                    let mut exited_pid = 0;
                    let mut kind = COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND::default();
                    unsafe { args.BrowserProcessId(&mut exited_pid)?; args.BrowserProcessExitKind(&mut kind)?; }
                    let mut state = events.lock().unwrap();
                    if state.original_pid != 0 && exited_pid == state.original_pid {
                        state.exited = true;
                        state.normal = Some(kind == COREWEBVIEW2_BROWSER_PROCESS_EXIT_KIND_NORMAL);
                    }
                    Ok(())
                }));
                let mut token = 0;
                unsafe { environment.add_BrowserProcessExited(&handler,&mut token)?; }
                REGISTRATIONS.with(|entries|entries.borrow_mut().insert(id,Registration {environment,token}));
                Ok(())
            })();
            let _ = tx.send(result.map_err(|_|"Profile native process observer unavailable".to_owned()));
        }).map_err(|_|"Profile process observer dispatch unavailable")?;
        rx.await.map_err(|_|"Profile process observer dispatch lost")??;
        Ok(Self {id,state})
    }
    pub fn snapshot(&self) -> Value {
        let state = self.state.lock().unwrap();
        json!({"original_browser_exit_seen":state.exited,"original_browser_exit_normal":state.normal})
    }
    pub async fn compare_reopened(&self, view: &tauri::Webview) -> Result<Value,String> {
        let (tx,rx) = tokio::sync::oneshot::channel();
        view.with_webview(move |platform| {
            let result = (|| -> ::windows::core::Result<u32> {
                let core = unsafe { platform.controller().CoreWebView2()? };
                let mut pid=0;
                unsafe {core.BrowserProcessId(&mut pid)?;}
                Ok(pid)
            })();
            let _=tx.send(result.map_err(|_|"Profile reopened process identity unavailable".to_owned()));
        }).map_err(|_|"Profile reopened process dispatch unavailable")?;
        let pid = rx.await.map_err(|_|"Profile reopened process dispatch lost")??;
        let state = self.state.lock().unwrap();
        Ok(json!({"old_pid_equals_reopened_pid":(pid!=0&&state.original_pid!=0).then_some(pid==state.original_pid),"original_browser_exit_seen":state.exited,"original_browser_exit_normal":state.normal}))
    }
    pub async fn close(self, app: &tauri::AppHandle) -> Result<(),String> {
        let (tx,rx)=tokio::sync::oneshot::channel();
        let id=self.id;
        app.run_on_main_thread(move || {
            let registration=REGISTRATIONS.with(|entries|entries.borrow_mut().remove(&id));
            let result=if let Some(registration)=registration {
                unsafe {registration.environment.remove_BrowserProcessExited(registration.token)}
                    .map_err(|_|"Profile process observer removal failed".to_owned())
            } else {Ok(())};
            let _=tx.send(result);
        }).map_err(|_|"Profile process observer cleanup dispatch unavailable")?;
        rx.await.map_err(|_|"Profile process observer cleanup dispatch lost")?
    }
}
