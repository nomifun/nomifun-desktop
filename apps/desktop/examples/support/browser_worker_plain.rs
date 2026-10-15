//! Controlled embedded WebView2 constructor comparison without Runtime owners.
//! It uses the same external-page builder contract and a disposable native UDF.
use serde_json::{Value,json};
use std::time::Duration;
use tauri::Manager;

pub(super) async fn verify(app:&tauri::AppHandle,fixture:&super::navigation_fixture::Fixture)->Result<Value,String> {
    let profile=tempfile::tempdir().map_err(|_|"Plain worker fixture profile unavailable")?;
    let directory=profile.path().to_path_buf();let handle=app.clone();
    let label=format!("plain-worker-{}",uuid::Uuid::now_v7());
    let view=tauri::async_runtime::spawn_blocking(move ||{
        handle.get_window("main").ok_or("Plain worker main window unavailable")?.add_child(
            tauri::webview::WebviewBuilder::new(label,tauri::WebviewUrl::External("about:blank".parse().unwrap()))
                .external_page().data_directory(directory).incognito(true).devtools(false).focused(false).disable_drag_drop_handler(),
            tauri::LogicalPosition::new(20.0,60.0),tauri::LogicalSize::new(1060.0,640.0)).map_err(|_|"Plain embedded worker constructor failed")
    }).await.map_err(|_|"Plain worker construction task failed")??;
    let result=async {
        let witness=super::worker_witness::Witness::install(&view).await?;
        view.show().map_err(|_|"Plain embedded worker show failed")?;
        super::windows::protocol_call(&view,"Page.navigate",json!({"url":fixture.url("/nav/resources-worker-plain")})).await?;
        let deadline=tokio::time::Instant::now()+Duration::from_secs(6);
        loop {
            let complete=super::evaluate(&view,"location.pathname==='/nav/resources-worker-plain' && document.readyState==='complete'").await.unwrap_or(Value::Null);
            if complete==true {break;}
            if tokio::time::Instant::now()>=deadline {return Err("Plain embedded worker page did not load".to_owned());}
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let deadline=tokio::time::Instant::now()+Duration::from_secs(5);
        let page=loop {
            let page=super::evaluate(&view,"({frame_count:fixtureFrames.length,worker_received:fixtureWorker!==null,worker:fixtureWorker,error:fixtureWorkerError,document_visibility:document.visibilityState,hidden:document.hidden,root:{isTauri:'isTauri' in window,internals:'__TAURI_INTERNALS__' in window,ipc:'ipc' in window,ua:navigator.userAgent}})").await?;
            if page["worker_received"]==true || tokio::time::Instant::now()>=deadline {break page;}
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        let native=witness.snapshot(&fixture.url("/nav/worker-plain.js")).await;
        drop(witness);
        Ok(json!({"embedded_webview2":true,"same_external_page_builder":true,"disposable_incognito_udf":true,
            "production_navigation_owner_installed":false,"fetch_navigation_gate_installed":false,"frame_sessions_installed":false,
            "worker_script_http_requests":fixture.count("/nav/worker-plain.js"),
            "worker_execution_http_side_effects":fixture.count("/nav/worker-plain-executed"),"page":page,"native":native}))
    }.await;
    let closed=super::windows::close_native_view(&view).await;
    drop(view);
    let cleanup=closed.and_then(|_|super::cleanup_fixture_profile(&profile));
    drop(profile);
    match (result,cleanup) {
        (Ok(mut value),Ok(()))=>{value["native_close_and_profile_cleanup"]=json!(true);Ok(value)}
        (Err(error),Ok(()))=>Err(error),
        (result,Err(error))=>Err(format!("Plain embedded worker comparison={result:?}; cleanup={error}")),
    }
}
