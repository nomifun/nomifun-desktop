//! Real user-command regression; two fresh owned persistent conversation Profiles.
use nomifun_browser_platform::{runtime::*, workspace::BrowserResourceService};
use serde_json::{Value, json};
use std::sync::Arc;
use tauri::Manager;

async fn evaluate(view:&tauri::Webview, expression:&str) -> Result<Value,String> {
    let result = super::windows::protocol_call(view,"Runtime.evaluate",json!({"expression":expression,"awaitPromise":true,"returnByValue":true})).await?;
    if result.get("exceptionDetails").is_some() { return Err(format!("Site-data fixture script failed: {result}")); }
    result["result"].get("value").cloned().ok_or_else(||"Fixture value missing".into())
}
const SEED: &str = r#"(async()=>{
document.cookie='nomi_site_probe=present; path=/; SameSite=Lax; Max-Age=3600';
localStorage.setItem('nomi_site_probe','present'); sessionStorage.setItem('nomi_site_probe','present');
await new Promise((resolve,reject)=>{const request=indexedDB.open('nomi-site-probe',1);request.onupgradeneeded=()=>request.result.createObjectStore('items');request.onerror=()=>reject(request.error);request.onsuccess=()=>{request.result.close();resolve();};});
const cache=await caches.open('nomi-site-probe');await cache.put('/probe-cached',new Response('owned fixture'));
await navigator.serviceWorker.register('/site-data-probe-worker.js');await navigator.serviceWorker.ready;
return true;
})()"#;
const READ: &str = r#"(async()=>({cookie:document.cookie,local:localStorage.getItem('nomi_site_probe'),session:sessionStorage.getItem('nomi_site_probe'),writer:localStorage.getItem('nomi_background_write'),databases:(await indexedDB.databases()).map(db=>db.name),caches:await caches.keys(),workers:(await navigator.serviceWorker.getRegistrations()).length}))()"#;
fn persisted_seed(value:&Value) -> bool {
    value["cookie"].as_str().is_some_and(|v|v.contains("nomi_site_probe=present"))
        && value["local"]=="present" && value["databases"]==json!(["nomi-site-probe"])
        && value["caches"]==json!(["nomi-site-probe"]) && value["workers"]==1
}

pub(super) async fn verify(app:&tauri::AppHandle,url:&str,data_root:&std::path::Path) -> Result<Value,String> {
    let service=BrowserResourceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let first_authority=super::browser_resource_fixture::authority("site-data-probe","cleared","native-site-data");
    let control_authority=super::browser_resource_fixture::authority("site-data-probe","untouched","native-site-data");
    let first_key=first_authority.key();let control_key=control_authority.key();
    let result=async {
        let first=service.ensure(first_authority,BrowserProfile::for_agent_session(data_root,&first_key,false)).await.map_err(|e|e.to_string())?;
        let control=service.ensure(control_authority,BrowserProfile::for_agent_session(data_root,&control_key,false)).await.map_err(|e|e.to_string())?;
        let clear=||BrowserTabCommand::ClearSiteData{runtime_generation:first.runtime_generation()};
        if first.user_command(clear()).await.is_ok() {return Err("Clear created an absent runtime".into());}
        let mut views=vec![];
        for workspace in [&first,&control] {
            workspace.user_command(BrowserTabCommand::Create{url:url.into()}).await.map_err(|e|e.to_string())?;
            let target=super::wait_workspace_page(workspace,url).await?;
            let view=app.get_webview(&target.tab_id).ok_or("Fixture view missing")?;
            evaluate(&view,SEED).await?;
            let data=evaluate(&view,READ).await?;
            if !persisted_seed(&data) || data["session"]!="present" {return Err("Fixture data was not seeded".into());}
            views.push(view);
        }
        if first.user_command(BrowserTabCommand::ClearSiteData{runtime_generation:first.runtime_generation()+1}).await.is_ok() {return Err("Stale clear was accepted".into());}
        let cancelled=tokio_util::sync::CancellationToken::new();cancelled.cancel();
        if super::windows::site_data::clear(&views[0],cancelled).await.is_ok() || !persisted_seed(&evaluate(&views[0],READ).await?) {
            return Err("Cancelled native dispatch changed site data".into());
        }
        let run=first.begin_run().await.map_err(|e|e.to_string())?;
        let agent_denied=first.agent_command(&run,clear()).await.is_err();
        let user_denied=first.user_command(clear()).await.is_err();
        first.finish_run(&run).await.map_err(|e|e.to_string())?;
        if !agent_denied || !user_denied || !persisted_seed(&evaluate(&views[0],READ).await?) {return Err("Run authority failed to protect site data".into());}
        first.user_command(BrowserTabCommand::CloseAll{runtime_generation:first.runtime_generation()}).await.map_err(|e|e.to_string())?;
        if super::windows::site_data::clear(&views[0],Default::default()).await.is_ok() {return Err("Closed view reported a successful clear".into());}
        first.user_command(BrowserTabCommand::Create{url:url.into()}).await.map_err(|e|e.to_string())?;
        let target=super::wait_workspace_page(&first,url).await?;
        views[0]=app.get_webview(&target.tab_id).ok_or("Persistence control view missing")?;
        let persisted=evaluate(&views[0],READ).await?;
        if !persisted["session"].is_null() || !persisted_seed(&persisted) {return Err(format!("Persistence control failed: {persisted}"));}
        // A second origin in the same conversation must also be cleared, even
        // while its document is actively writing data before native teardown.
        let mut other_origin=url::Url::parse(url).map_err(|e|e.to_string())?;
        other_origin.set_host(Some("localhost")).map_err(|e|e.to_string())?;
        first.user_command(BrowserTabCommand::Create{url:other_origin.to_string()}).await.map_err(|e|e.to_string())?;
        let writer_target=super::wait_workspace_page(&first,other_origin.as_str()).await?;
        let writer=app.get_webview(&writer_target.tab_id).ok_or("Second-origin writer missing")?;
        evaluate(&writer,SEED).await?;
        evaluate(&writer,"(async()=>{setInterval(()=>localStorage.setItem('nomi_background_write','active'),10);while(localStorage.getItem('nomi_background_write')!=='active')await new Promise(resolve=>setTimeout(resolve,10));return true;})()").await?;
        if !persisted_seed(&evaluate(&writer,READ).await?) {return Err("Second origin was not seeded".into());}
        let cleared_snapshot=first.user_command(clear()).await.map_err(|e|e.to_string())?;
        if !cleared_snapshot.tabs.is_empty() || app.get_webview(views[0].label()).is_some() || app.get_webview(writer.label()).is_some() {return Err("Clear did not close every original page".into());}
        if app.webviews().keys().any(|label|label.starts_with("browser-site-data-")) {return Err("Clear left a maintenance controller behind".into());}
        if first.user_command(clear()).await.is_ok() {return Err("Empty-page clear unexpectedly created a maintenance controller".into());}
        first.user_command(BrowserTabCommand::Create{url:url.into()}).await.map_err(|e|e.to_string())?;
        let target=super::wait_workspace_page(&first,url).await?;
        let fresh=app.get_webview(&target.tab_id).ok_or("Reopened fixture missing")?;
        let cleared=evaluate(&fresh,READ).await?;
        let empty=json!({"cookie":"","local":null,"session":null,"writer":null,"databases":[],"caches":[],"workers":0});
        if cleared!=empty {return Err(format!("Site data remained after clear: {cleared}"));}
        first.user_command(BrowserTabCommand::Create{url:other_origin.to_string()}).await.map_err(|e|e.to_string())?;
        let target=super::wait_workspace_page(&first,other_origin.as_str()).await?;
        let fresh=app.get_webview(&target.tab_id).ok_or("Reopened second origin missing")?;
        let other_cleared=evaluate(&fresh,READ).await?;
        if other_cleared!=empty {return Err(format!("Second-origin data or writer survived clear: {other_cleared}"));}
        let control_data=evaluate(&views[1],READ).await?;
        if !persisted_seed(&control_data) || control_data["session"]!="present" {return Err("Other conversation was affected".into());}
        Ok(json!({"production_user_command":true,"stale_absent_and_agent_run_rejected":true,"cancel_before_dispatch_preserves_data":true,"closed_view_cannot_report_success":true,"persistent_data_survived_ordinary_close":true,"native_clear_and_controller_close_acknowledged":true,"cookies_local_session_indexeddb_cache_service_workers_cleared":true,"two_origins_and_active_writer_cleared":true,"other_conversation_unchanged":true}))
    }.await;
    let first_closed=service.close(&first_key).await.map_err(|e|e.to_string());
    let control_closed=service.close(&control_key).await.map_err(|e|e.to_string());
    first_closed?;control_closed?;
    result
}
