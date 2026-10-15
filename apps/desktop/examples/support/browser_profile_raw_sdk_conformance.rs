//! Isolated public-SDK constructor/Close diagnostic on the existing Tauri HWND.
//! Native COM objects stay on the UI thread; no Wry Core, IPC or DevTools session.
use serde_json::{Value, json};
use std::{cell::RefCell, collections::BTreeMap, path::Path, rc::Rc, time::{Duration, Instant}};
use std::os::windows::{ffi::OsStrExt,fs::OpenOptionsExt,io::AsRawHandle};
use tauri::Manager;
use webview2_com::{CoreWebView2EnvironmentOptions, CoTaskMemPWSTR,
    CreateCoreWebView2EnvironmentCompletedHandler, CreateCoreWebView2ControllerCompletedHandler,
    ExecuteScriptCompletedHandler, Microsoft::Web::WebView2::Win32::{
        CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2Environment,
        ICoreWebView2EnvironmentOptions, ICoreWebView2Environment7, ICoreWebView2Environment10,
        ICoreWebView2Controller, ICoreWebView2, ICoreWebView2_2, ICoreWebView2_13,
    }};
use ::windows::{core::{BOOL, HSTRING, Interface, PCWSTR, PWSTR}, Win32::{Foundation::{RECT,HANDLE},Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION,FILE_ATTRIBUTE_DIRECTORY,FILE_FLAG_BACKUP_SEMANTICS,FILE_READ_ATTRIBUTES,FILE_SHARE_DELETE,FILE_SHARE_READ,FILE_SHARE_WRITE,GetFileInformationByHandle},System::Com::{CoInitializeEx,CoUninitialize,COINIT_APARTMENTTHREADED}}};

fn same_directory_objects(expected:&Path,actual:&Path)->Result<(),String>{
    let identity=|path:&Path|->Result<(std::fs::File,(u32,u64)),String>{
        if !path.is_absolute(){return Err("Raw SDK directory selection is not absolute".into());}
        let file=std::fs::OpenOptions::new().access_mode(FILE_READ_ATTRIBUTES.0)
            .share_mode((FILE_SHARE_READ|FILE_SHARE_WRITE|FILE_SHARE_DELETE).0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0).open(path).map_err(|_|"Raw SDK directory identity unavailable")?;
        let mut info=BY_HANDLE_FILE_INFORMATION::default();
        unsafe{GetFileInformationByHandle(HANDLE(file.as_raw_handle()),&mut info)}.map_err(|_|"Raw SDK directory file identity unavailable")?;
        if info.dwFileAttributes&FILE_ATTRIBUTE_DIRECTORY.0==0{return Err("Raw SDK selected object is not a directory".into());}
        Ok((file,(info.dwVolumeSerialNumber,((info.nFileIndexHigh as u64)<<32)|info.nFileIndexLow as u64)))
    };
    // Keep both directory handles pinned until comparison completes. No file
    // identity or path is exported, only the successful guard boolean.
    let (_expected_handle,expected_id)=identity(expected)?;let (_actual_handle,actual_id)=identity(actual)?;
    if expected_id!=actual_id{return Err("Raw SDK directory objects differ".into());}Ok(())
}

struct Apartment { _ui_thread:std::marker::PhantomData<Rc<()>> }
impl Drop for Apartment {fn drop(&mut self){unsafe{CoUninitialize()};}}
struct Registration { environment:ICoreWebView2Environment, controller:ICoreWebView2Controller, close_attempted:bool, _apartment:Apartment }
thread_local! {static VIEWS:RefCell<BTreeMap<uuid::Uuid,Registration>>=RefCell::default();}
struct View {id:uuid::Uuid,app:tauri::AppHandle,input_metadata:Value}
impl Drop for View {fn drop(&mut self){let id=self.id;let _=self.app.run_on_main_thread(move||{let _=close_id(id);});}}
type CreationSender=Rc<RefCell<Option<tokio::sync::oneshot::Sender<Result<View,String>>>>>;
fn complete(sender:&CreationSender,result:Result<View,String>)->bool{sender.borrow_mut().take().is_some_and(|sender|sender.send(result).is_ok())}
fn close_id(id:uuid::Uuid)->Result<(),String>{
    VIEWS.with(|views|{
        let Some(mut view)=views.borrow_mut().remove(&id)else{return Ok(());};
        if view.close_attempted{
            views.borrow_mut().insert(id,view);
            return Err("Raw SDK controller Close already failed; no second attempt".into());
        }
        // This controller is not owned by Wry; this is its only Close.
        view.close_attempted=true;
        let result=unsafe{view.controller.Close()}.map_err(|_|"Raw SDK controller Close failed".to_owned());
        if result.is_err(){views.borrow_mut().insert(id,view);}result
    })
}
fn core(id:uuid::Uuid)->Result<ICoreWebView2,String>{
    VIEWS.with(|views|views.borrow().get(&id).ok_or("Raw SDK controller absent").and_then(|view|unsafe{view.controller.CoreWebView2()}.map_err(|_|"Raw SDK Core unavailable"))).map_err(str::to_owned)
}
fn native_string(raw:PWSTR,bound:usize)->Option<String>{
    let allocation=CoTaskMemPWSTR::from(raw);
    let value=if raw.is_null(){None}else{let text=unsafe{raw.as_wide()};(text.len()<=bound).then(||String::from_utf16_lossy(text))};
    drop(allocation);value
}

impl View {
    async fn create(app:&tauri::AppHandle,directory:&Path,normalize_input:bool)->Result<Self,String>{
        std::fs::create_dir_all(directory).map_err(|_|"Raw SDK owned UDF creation failed")?;
        // This changes only the API path representation. The inspect guard still
        // compares the selected native folder to the original intended folder.
        let input=if normalize_input{dunce::canonicalize(directory).map_err(|_|"Raw SDK normalized owned UDF unavailable")?}else{directory.to_path_buf()};
        same_directory_objects(directory,&input)?;
        let input_metadata=json!({"input_is_verbatim":input.as_os_str().to_string_lossy().starts_with(r"\\?\"),"input_length_utf16":input.as_os_str().encode_wide().count(),"normalized_input_requested":normalize_input});
        if normalize_input&&input_metadata["input_is_verbatim"]!=false{return Err("Raw SDK normalized input remains verbatim".into());}
        let id=uuid::Uuid::now_v7();let directory=HSTRING::from(input.as_os_str());
        let (tx,rx)=tokio::sync::oneshot::channel();let native_app=app.clone();
        app.run_on_main_thread(move||{
            let sender=Rc::new(RefCell::new(Some(tx)));let environment_sender=sender.clone();
            let result=(||->Result<(),String>{
                unsafe{CoInitializeEx(None,COINIT_APARTMENTTHREADED)}.ok().map_err(|_|"Raw SDK STA COM initialization failed")?;
                let apartment=Apartment{_ui_thread:std::marker::PhantomData};
                let hwnd=native_app.get_window("main").ok_or("Raw SDK Tauri parent absent")?.hwnd().map_err(|_|"Raw SDK parent HWND unavailable")?;
                let handler=CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move|error,environment|{
                    let result=(||->Result<(),String>{
                        error.map_err(|_|"Raw SDK environment creation failed")?;
                        let environment=environment.ok_or("Raw SDK environment missing")?;
                        let owner=environment.clone();let controller_sender=environment_sender.clone();let controller_app=native_app.clone();
                        let completed=CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move|error,controller|{
                            let result=(||->Result<View,String>{
                                error.map_err(|_|"Raw SDK controller creation failed")?;
                                let controller=controller.ok_or("Raw SDK controller missing")?;
                                if controller_sender.borrow().as_ref().is_none_or(|sender|sender.is_closed()) {
                                    let _=unsafe{controller.Close()};
                                    return Err("Raw SDK controller callback lost its caller".into());
                                }
                                // Match the previous offscreen embedded constructor geometry.
                                if unsafe{controller.SetBounds(RECT{left:-32000,top:-32000,right:-30976,bottom:-31232})}.is_err(){
                                    let _=unsafe{controller.Close()};
                                    return Err("Raw SDK controller bounds failed".into());
                                }
                                VIEWS.with(|views|views.borrow_mut().insert(id,Registration{environment:owner,controller,close_attempted:false,_apartment:apartment}));
                                Ok(View{id,app:controller_app,input_metadata})
                            })();
                            let created=result.is_ok();
                            if !complete(&controller_sender,result)&&created{let _=close_id(id);}Ok(())
                        }));
                        let environment10=environment.cast::<ICoreWebView2Environment10>().map_err(|_|"Raw SDK controller-options interface unavailable")?;
                        let options=unsafe{environment10.CreateCoreWebView2ControllerOptions()}.map_err(|_|"Raw SDK controller options unavailable")?;
                        // Wry leaves ProfileName at the SDK default empty selection.
                        unsafe{options.SetIsInPrivateModeEnabled(false)}.map_err(|_|"Raw SDK persistent controller selection failed")?;
                        unsafe{environment10.CreateCoreWebView2ControllerWithOptions(hwnd,&options,&completed)}.map_err(|_|"Raw SDK controller dispatch failed".to_owned())
                    })();
                    if let Err(error)=result{complete(&environment_sender,Err(error));}Ok(())
                }));
                let options=ICoreWebView2EnvironmentOptions::from(CoreWebView2EnvironmentOptions::default());
                unsafe{CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(),&directory,&options,&handler)}.map_err(|_|"Raw SDK environment dispatch failed".into())
            })();
            if let Err(error)=result{complete(&sender,Err(error));}
        }).map_err(|_|"Raw SDK create UI dispatch unavailable")?;
        tokio::time::timeout(Duration::from_secs(15),rx).await.map_err(|_|"Raw SDK create callback deadline")?
            .map_err(|_|"Raw SDK create callback lost".to_owned())?
    }
    async fn inspect(&self,app:&tauri::AppHandle,directory:&Path)->Result<(super::process_witness::Discriminator,Value),String>{
        let id=self.id;let directory=directory.to_path_buf();let (tx,rx)=tokio::sync::oneshot::channel();
        app.run_on_main_thread(move||{
            let result=(||->Result<_,String>{
                let environment=VIEWS.with(|views|views.borrow().get(&id).map(|view|view.environment.clone())).ok_or("Raw SDK environment registration absent")?;
                let environment7=environment.cast::<ICoreWebView2Environment7>().map_err(|_|"Raw SDK UDF getter unavailable")?;
                let mut raw=PWSTR::null();let received=unsafe{environment7.UserDataFolder(&mut raw)};let actual=native_string(raw,32768);
                received.map_err(|_|"Raw SDK UDF unavailable")?;
                let actual=std::path::PathBuf::from(actual.ok_or("Raw SDK UDF invalid")?);
                let actual_is_absolute=actual.is_absolute();
                let actual_is_verbatim=matches!(actual.components().next(),Some(std::path::Component::Prefix(prefix))
                    if matches!(prefix.kind(),std::path::Prefix::VerbatimDisk(_)|std::path::Prefix::VerbatimUNC(_,_)|std::path::Prefix::Verbatim(_)));
                let actual_is_ordinary_disk=matches!(actual.components().next(),Some(std::path::Component::Prefix(prefix))
                    if matches!(prefix.kind(),std::path::Prefix::Disk(_)));
                let actual=dunce::canonicalize(actual).map_err(|_|"Raw SDK actual UDF inaccessible")?;
                let expected=dunce::canonicalize(&directory).map_err(|_|"Raw SDK owned UDF inaccessible")?;
                same_directory_objects(&directory,&actual)?;
                if actual!=expected{return Err("Raw SDK native UDF differs from owned selection".into());}
                let core=core(id)?;let profile=unsafe{core.cast::<ICoreWebView2_13>().map_err(|_|"Raw SDK Profile getter unavailable")?.Profile()}.map_err(|_|"Raw SDK Profile unavailable")?;
                let mut private=BOOL::default();unsafe{profile.IsInPrivateModeEnabled(&mut private)}.map_err(|_|"Raw SDK private-mode getter unavailable")?;
                if private.as_bool(){return Err("Raw SDK did not select a persistent native Profile".into());}
                let identity=super::process_witness::discriminate_core(&core)?;
                if identity.path_is_within(&directory)!=Some(true){return Err("Raw SDK native Profile path is outside the owned UDF".into());}
                let mut raw=PWSTR::null();let received=unsafe{environment.BrowserVersionString(&mut raw)};let version=native_string(raw,80);
                let version=received.is_ok().then_some(version).flatten().filter(|version|version.bytes().all(|byte|byte.is_ascii_digit()||byte==b'.'));
                Ok((identity,json!({"native_udf_matches_owned_selection":true,"native_udf_directory_object_matches_owned_selection":true,"native_udf_getter_is_absolute":actual_is_absolute,"native_udf_getter_is_verbatim":actual_is_verbatim,"native_udf_getter_is_ordinary_disk":actual_is_ordinary_disk,"native_is_in_private":false,"native_profile_path_is_within_owned_udf":true,"browser_version":version})))
            })();let _=tx.send(result);
        }).map_err(|_|"Raw SDK inspect UI dispatch unavailable")?;
        rx.await.map_err(|_|"Raw SDK inspect callback lost")?
    }
    async fn cookies(&self,app:&tauri::AppHandle)->Result<Value,String>{
        let id=self.id;let (tx,rx)=tokio::sync::oneshot::channel();
        app.run_on_main_thread(move||{
            let manager=core(id).and_then(|core|unsafe{core.cast::<ICoreWebView2_2>().map_err(|_|"Raw SDK CookieManager interface unavailable")?.CookieManager()}.map_err(|_|"Raw SDK CookieManager getter failed".to_owned()));
            match manager{Ok(manager)=>super::process_witness::dispatch_cookie_inventory(&manager,tx),Err(error)=>{let _=tx.send(Err(error));}}
        }).map_err(|_|"Raw SDK Cookie inventory UI dispatch unavailable")?;
        tokio::time::timeout(Duration::from_secs(5),rx).await.map_err(|_|"Raw SDK Cookie inventory callback deadline")?.map_err(|_|"Raw SDK Cookie inventory callback lost")?
    }
    async fn execute(&self,app:&tauri::AppHandle)->Result<Value,String>{
        let id=self.id;let (tx,rx)=tokio::sync::oneshot::channel();
        app.run_on_main_thread(move||{
            let sender=Rc::new(RefCell::new(Some(tx)));let completed=sender.clone();
            let result=(||->Result<(),String>{
                let handler=ExecuteScriptCompletedHandler::create(Box::new(move|error,text|{
                    let result=if error.is_err()||text.is_empty()||text.len()>8192{Err("Raw SDK script result unavailable".to_owned())}else{serde_json::from_str(&text).map_err(|_|"Raw SDK script result invalid".to_owned())};
                    if let Some(sender)=completed.borrow_mut().take(){let _=sender.send(result);}Ok(())
                }));
                unsafe{core(id)?.ExecuteScript(&HSTRING::from("window.profileConformanceSettled || null"),&handler)}.map_err(|_|"Raw SDK script dispatch failed".into())
            })();
            if let Err(error)=result{if let Some(sender)=sender.borrow_mut().take(){let _=sender.send(Err(error));}}
        }).map_err(|_|"Raw SDK script UI dispatch unavailable")?;
        tokio::time::timeout(Duration::from_secs(5),rx).await.map_err(|_|"Raw SDK script callback deadline")?.map_err(|_|"Raw SDK script callback lost")?
    }
    async fn navigate(&self,app:&tauri::AppHandle,url:&str)->Result<Value,String>{
        let id=self.id;let url=HSTRING::from(url);let (tx,rx)=tokio::sync::oneshot::channel();
        app.run_on_main_thread(move||{let result=core(id).and_then(|core|unsafe{core.Navigate(&url)}.map_err(|_|"Raw SDK Navigate failed".into()));let _=tx.send(result);}).map_err(|_|"Raw SDK Navigate UI dispatch unavailable")?;
        rx.await.map_err(|_|"Raw SDK Navigate callback lost")??;
        let deadline=Instant::now()+Duration::from_secs(10);
        loop{let result=self.execute(app).await?;if result.is_object(){return Ok(result);}if Instant::now()>=deadline{return Err("Raw SDK real storage promise did not settle".into());}tokio::time::sleep(Duration::from_millis(25)).await;}
    }
    async fn close(&self,app:&tauri::AppHandle)->Result<(),String>{
        let id=self.id;let (tx,rx)=tokio::sync::oneshot::channel();
        app.run_on_main_thread(move||{
            let _=tx.send(close_id(id));
        }).map_err(|_|"Raw SDK close UI dispatch unavailable")?;
        rx.await.map_err(|_|"Raw SDK close callback lost")?
    }
}

pub(super) async fn verify(app:&tauri::AppHandle,directory:&Path,url:&str,normalize_input:bool)->Result<Value,String>{
    let first=View::create(app,directory,normalize_input).await?;
    let result=async{
        let (identity_before,runtime_before)=first.inspect(app,directory).await?;
        let before=first.navigate(app,url).await?;let sdk_before=first.cookies(app).await?;
        let database_before=identity_before.cookie_database_exists_within(directory);
        first.close(app).await?;
        let reopened=View::create(app,directory,normalize_input).await?;
        let result=async{
            let (identity_after,runtime_after)=reopened.inspect(app,directory).await?;
            let mut target=url::Url::parse(url).map_err(|_|"Raw SDK target invalid")?;target.set_path("/profile-reopen");
            let after=reopened.navigate(app,target.as_str()).await?;let sdk_after=reopened.cookies(app).await?;
            let database_after=identity_after.cookie_database_exists_within(directory);
            Ok(json!({"diagnostic_only":true,"constructor":"raw_public_sdk","native_controller_profile_selection":"sdk_default","native_public_api_only":true,"no_wry_core_ipc_or_cdp":true,"controller_close_once_per_instance":true,"native_udf_input_before":first.input_metadata.clone(),"native_udf_input_after":reopened.input_metadata.clone(),
                "initial_http_cookie":before["cookie"].as_bool(),"reopened_http_cookie":after["cookie"].as_bool(),
                "initial_local":before["local"].as_bool(),"reopened_local":after["local"].as_bool(),"initial_indexeddb":before["indexeddb"].as_bool(),"reopened_indexeddb":after["indexeddb"].as_bool(),
                "cookie_database_file_exists_before_close":database_before,"cookie_database_file_exists_after_reopen":database_after,
                "public_sdk_before":sdk_before,"public_sdk_after":sdk_after,"native_identity":identity_before.compare(&identity_after),"runtime_before":runtime_before,"runtime_after":runtime_after}))
        }.await;
        reopened.close(app).await?;result
    }.await;
    first.close(app).await?;result
}
