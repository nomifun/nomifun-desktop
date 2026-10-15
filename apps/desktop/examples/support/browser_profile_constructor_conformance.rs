//! Controlled plain-Wry constructor comparison. No DevTools or host handlers.
use serde_json::{Value,json};
use std::{cell::RefCell,path::{Path,PathBuf},rc::Rc,time::{Duration,Instant}};
use tauri::Manager;
use webview2_com::ExecuteScriptCompletedHandler;
use ::windows::core::HSTRING;

async fn execute(view:&tauri::Webview,expression:&str)->Result<Value,String> {
    let expression=HSTRING::from(expression);
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let sender=Rc::new(RefCell::new(Some(tx)));
        let completed=sender.clone();
        let result=(||->::windows::core::Result<()> {
            let core=unsafe{platform.controller().CoreWebView2()?};
            let handler=ExecuteScriptCompletedHandler::create(Box::new(move |error,text| {
                let result=if error.is_err()||text.is_empty(){Err("Plain constructor script result unavailable".to_owned())}else{
                    if text.len()>8192{Err("Plain constructor script result overflow".to_owned())}else{
                        serde_json::from_str(&text).map_err(|_|"Plain constructor script result invalid".to_owned())
                    }
                };
                if let Some(sender)=completed.borrow_mut().take(){let _=sender.send(result);}
                Ok(())
            }));
            unsafe{core.ExecuteScript(&expression,&handler)?;}
            Ok(())
        })();
        if result.is_err(){if let Some(sender)=sender.borrow_mut().take(){let _=sender.send(Err("Plain constructor script dispatch failed".into()));}}
    }).map_err(|_|"Plain constructor native script dispatch unavailable")?;
    rx.await.map_err(|_|"Plain constructor native script callback lost")?
}

async fn navigate(view:&tauri::Webview,url:&str)->Result<Value,String> {
    let address=HSTRING::from(url);
    let (tx,rx)=tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result=(||->::windows::core::Result<()> {unsafe{platform.controller().CoreWebView2()?.Navigate(&address)}})();
        let _=tx.send(result.map_err(|_|"Plain constructor native Navigate failed".to_owned()));
    }).map_err(|_|"Plain constructor native Navigate dispatch unavailable")?;
    rx.await.map_err(|_|"Plain constructor native Navigate callback lost")??;
    let deadline=Instant::now()+Duration::from_secs(10);
    loop {
        let result=execute(view,"window.profileConformanceSettled || null").await?;
        if result.is_object(){return Ok(result);}
        if Instant::now()>=deadline{return Err("Plain constructor real storage promise did not settle".into());}
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn create(app:&tauri::AppHandle,directory:PathBuf,external:bool)->Result<tauri::Webview,String> {
    let label=format!("plain-profile-{}",uuid::Uuid::now_v7());
    let app=app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let mut builder=tauri::webview::WebviewBuilder::new(label,tauri::WebviewUrl::External("about:blank".parse().unwrap()))
            .data_directory(directory).incognito(false).devtools(false).focused(false).disable_drag_drop_handler();
        if external{builder=builder.external_page();}
        app.get_window("main").ok_or("Plain constructor native parent absent")?.add_child(builder,
            tauri::LogicalPosition::new(-32000.0,-32000.0),tauri::LogicalSize::new(1024.0,768.0))
            .map_err(|_|"Plain constructor child creation failed")
    }).await.map_err(|_|"Plain constructor worker failed")?.map_err(str::to_owned)
}

pub(super) async fn verify(app:&tauri::AppHandle,directory:&Path,url:&str,external:bool,retain_manager:bool)->Result<Value,String> {
    let first=create(app,directory.to_path_buf(),external).await?;
    let mut retained=None;
    let result=async {
        super::super::windows::effective_environment::inspect(&first,directory.to_path_buf(),nomifun_browser_platform::runtime::BrowserProfilePersistence::Persistent).await.map_err(|error|error.code().to_owned())?;
        let before=navigate(&first,url).await?;
        let identity_before=super::process_witness::discriminate(&first).await?;
        if identity_before.path_is_within(directory)!=Some(true){return Err("Plain constructor Profile path is outside the owned UDF".into());}
        let sdk_before=super::process_witness::cookie_manager_witness(&first).await?;
        if retain_manager{retained=Some(super::process_witness::CookieManagerIntervention::install(&first).await?);}
        super::super::windows::close_native_view(&first).await?;
        let old_after_close=if let Some(manager)=&retained{manager.snapshot(app).await}else{Value::Null};
        let reopened=create(app,directory.to_path_buf(),external).await?;
        let second_result=async {
            super::super::windows::effective_environment::inspect(&reopened,directory.to_path_buf(),nomifun_browser_platform::runtime::BrowserProfilePersistence::Persistent).await.map_err(|error|error.code().to_owned())?;
            let mut target=url::Url::parse(url).map_err(|_|"Plain constructor target invalid")?;target.set_path("/profile-reopen");
            let after=navigate(&reopened,target.as_str()).await?;
            let identity_after=super::process_witness::discriminate(&reopened).await?;
            if identity_after.path_is_within(directory)!=Some(true){return Err("Reopened plain constructor Profile path is outside the owned UDF".into());}
            let sdk_after=super::process_witness::cookie_manager_witness(&reopened).await?;
            let old_after_reopen=if let Some(manager)=&retained{manager.snapshot(app).await}else{Value::Null};
            Ok(json!({"diagnostic_only":true,"constructor":if retain_manager{"plain_external_page_with_retained_profile_cookie_manager"}else if external{"plain_external_page"}else{"plain_legacy_tauri"},"native_public_api_only":true,"no_production_handlers_or_cdp":true,
                "controlled_profile_cookie_manager_lifetime_intervention":retain_manager,"old_public_sdk_after_close":old_after_close,"old_public_sdk_after_reopen":old_after_reopen,
                "initial_http_cookie":before["cookie"].as_bool(),"reopened_http_cookie":after["cookie"].as_bool(),
                "initial_local":before["local"].as_bool(),"reopened_local":after["local"].as_bool(),
                "initial_indexeddb":before["indexeddb"].as_bool(),"reopened_indexeddb":after["indexeddb"].as_bool(),
                "public_sdk_before":sdk_before,"public_sdk_after":sdk_after,"native_identity":identity_before.compare(&identity_after),
                "native_profile_path_is_within_owned_udf":identity_after.path_is_within(directory)}))
        }.await;
        super::super::windows::close_native_view(&reopened).await?;
        second_result
    }.await;
    if app.get_webview(first.label()).is_some(){super::super::windows::close_native_view(&first).await?;}
    if let Some(manager)=retained{manager.close(app).await?;}
    result
}
