//! Bounded real same-document and cross-document native history acceptance.
//! Independent HTTP receipts and exact native traces are the witnesses.
use super::navigation_fixture::Fixture;
use nomifun_browser_platform::runtime::*;
use nomifun_browser_platform::workspace::{BrowserResource,BrowserResourceService};
use serde_json::{Value,json};
use std::{sync::Arc,time::{Duration,Instant}};
use tauri::Manager;

fn error(value:impl std::fmt::Display)->String{value.to_string()}

pub(super) async fn verify(app:&tauri::AppHandle)->Result<Value,String>{
    let fixture=Fixture::new()?;
    let service=BrowserResourceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let authority=super::browser_resource_fixture::authority("native-history-fixture","same-document-history","native-history-provider");
    let key=authority.key();
    let runtime=service.ensure(authority,BrowserProfile::Ephemeral).await.map_err(error)?;
    let result=tokio::time::timeout(Duration::from_secs(35),scenarios(app,&fixture,&runtime)).await
        .map_err(|_|"Bounded native history fixture timed out".to_owned()).and_then(|value|value);
    if result.is_err(){
        if let Ok(snapshot)=runtime.snapshot().await{for tab in snapshot.runtime.into_iter().flat_map(|runtime|runtime.tabs){
            if let Ok(report)=runtime.navigation_diagnostics(tab.target).await{
                eprintln!("BROWSER_HISTORY_FAILURE_EVIDENCE {}",serde_json::to_string(&report).map_err(error)?);
            }
        }}
    }
    let cleanup=service.close(&key).await.map_err(error).and(service.shutdown().await.map_err(error));
    match(result,cleanup){(Ok(value),Ok(()))=>Ok(value),(Err(value),Ok(()))=>Err(value),(Ok(_),Err(value))=>Err(value),
        (Err(value),Err(cleanup))=>Err(format!("{value}; native history cleanup: {cleanup}"))}
}

async fn scenarios(app:&tauri::AppHandle,fixture:&Fixture,runtime:&Arc<BrowserResource>)->Result<Value,String>{
    runtime.set_surface(BrowserSurfaceBounds{x:20.0,y:60.0,width:1000.0,height:600.0},true,Default::default()).await.map_err(error)?;
    let created=runtime.user_command(BrowserTabCommand::Create{url:fixture.url("/nav/history-base")}).await.map_err(error)?;
    let id=created.active_tab_id.ok_or("Native history first page missing")?;
    let view=app.get_webview(&id).ok_or("Native history controller missing")?;
    let initial=finished(runtime,&id,&fixture.url("/nav/history-base")).await?;
    let nonce=super::evaluate(&view,"fixtureDocumentNonce").await?;
    let generation=initial.target.document_generation;
    let initial_sequence=load(&initial)?.navigation_sequence;
    let before=report(runtime,&id).await?;
    for (selector,path,index) in [("#history-one","/nav/history-spa-1",1),("#history-two","/nav/history-spa-2",2)]{
        super::native_fixture_click(&view,selector).await?;
        let tab=finished(runtime,&id,&fixture.url(path)).await?;
        if tab.target.document_generation!=generation || load(&tab)?.navigation_sequence!=initial_sequence
            || super::evaluate(&view,"fixtureDocumentNonce").await?!=nonce{
            return Err("Trusted pushState changed the real document or fabricated a native main attempt".into());
        }
        let input=super::evaluate(&view,"fixtureHistoryClicks").await?;
        if input.as_array().is_none_or(|input|input.len()!=index||input.iter().any(|input|input["trusted"]!=true)){
            return Err("Native history SPA setup lacks exact trusted input".into());
        }
    }
    let after_push=report(runtime,&id).await?;
    no_native_document_transition(&after_push,&before)?;
    let count_before=http_document_count(fixture);
    let mut same_document=Vec::new();
    for (back,path) in [(true,"/nav/history-spa-1"),(true,"/nav/history-base"),(false,"/nav/history-spa-1"),(false,"/nav/history-spa-2")]{
        let prior=tab(runtime,&id).await?;let before=report(runtime,&id).await?;
        let prior_fence=load(&prior)?.navigation_sequence;
        let command=if back{BrowserTabCommand::Back{target:prior.target}}else{BrowserTabCommand::Forward{target:prior.target}};
        runtime.user_command(command).await.map_err(error)?;
        let settled=finished(runtime,&id,&fixture.url(path)).await?;
        let after=report(runtime,&id).await?;
        if settled.target.document_generation!=generation || load(&settled)?.navigation_sequence<=prior_fence
            || super::evaluate(&view,"fixtureDocumentNonce").await?!=nonce || http_document_count(fixture)!=count_before{
            return Err("Same-document native Back/Forward replaced the document or sent another main HTTP request".into());
        }
        no_native_document_transition(&after,&before)?;
        if host_commands(&after)!=host_commands(&before)+1{return Err("Native history control audit lacks one exact host dispatch".into());}
        same_document.push(json!({"direction":if back{"back"}else{"forward"},"path":path,"phase":load(&settled)?.phase,
            "lifecycle":settled.lifecycle,"content_state":load(&settled)?.content_state,"estimated_progress":load(&settled)?.estimated_progress,
            "document_generation_unchanged":true,"history_control_fence_advanced":true,"document_nonce_unchanged":true,"main_http_delta":0,"native_start_commit_finish_delta":0,"host_command_delta":1}));
    }
    let locked_before=report(runtime,&id).await?;
    let locked_target=tab(runtime,&id).await?.target;
    let run=runtime.begin_run().await.map_err(error)?;
    let blocked=runtime.user_command(BrowserTabCommand::Back{target:locked_target}).await;
    runtime.finish_run(&run).await.map_err(error)?;
    if blocked!=Err(WorkspaceError::Admission(nomifun_browser_platform::run_guard::RunAdmissionError::UserInputLocked))
        || host_commands(&report(runtime,&id).await?)!=host_commands(&locked_before)
        || http_document_count(fixture)!=count_before{
        return Err("User history control escaped the real Agent run input lock or dispatched before rejection".into());
    }
    let reload_source=tab(runtime,&id).await?;let reload_before=report(runtime,&id).await?;
    let reload_count=http_document_count(fixture);let reload_sequence=load(&reload_source)?.navigation_sequence;
    let stale_reload=runtime.user_command(BrowserTabCommand::Reload{target:reload_source.target.clone(),expected_navigation_sequence:initial_sequence}).await;
    if stale_reload!=Err(WorkspaceError::StaleTarget)||host_commands(&report(runtime,&id).await?)!=host_commands(&reload_before)
        || http_document_count(fixture)!=reload_count{
        return Err("Old pre-history wire sequence affected a later same-document control fence".into());
    }
    runtime.user_command(BrowserTabCommand::Reload{target:reload_source.target,expected_navigation_sequence:reload_sequence}).await.map_err(error)?;
    let reloaded=finished(runtime,&id,&fixture.url("/nav/history-spa-2")).await?;
    let reload_after=report(runtime,&id).await?;
    let reload_events=since(&reload_after,&reload_before);
    if http_document_count(fixture)!=reload_count+1||reloaded.target.document_generation<=generation
        || reload_events.iter().filter(|event|event.kind==BrowserNavigationEventKind::Started).count()!=1
        || reload_events.iter().filter(|event|event.kind==BrowserNavigationEventKind::Finished).count()!=1{
        return Err("The current history control fence after SPA traversal could not authorize one explicit native Reload".into());
    }
    let source=tab(runtime,&id).await?;
    runtime.user_command(BrowserTabCommand::Navigate{target:source.target,url:fixture.url("/nav/history-other")}).await.map_err(error)?;
    let other=finished(runtime,&id,&fixture.url("/nav/history-other")).await?;
    if other.target.document_generation<=generation{return Err("Explicit other document did not advance real document generation".into());}
    let mut cross_document=Vec::new();
    for (back,path) in [(true,"/nav/history-spa-2"),(false,"/nav/history-other")]{
        let prior=tab(runtime,&id).await?;let before=report(runtime,&id).await?;let count=http_document_count(fixture);
        let previous_generation=prior.target.document_generation;
        let command=if back{BrowserTabCommand::Back{target:prior.target}}else{BrowserTabCommand::Forward{target:prior.target}};
        runtime.user_command(command).await.map_err(error)?;
        let settled=finished(runtime,&id,&fixture.url(path)).await?;let after=report(runtime,&id).await?;
        let sequence=load(&settled)?.navigation_sequence;
        let events=since(&after,&before);
        let starts:Vec<_>=events.iter().filter(|event|event.kind==BrowserNavigationEventKind::Started).collect();
        let finishes:Vec<_>=events.iter().filter(|event|event.kind==BrowserNavigationEventKind::Finished).collect();
        if starts.len()!=1||finishes.len()!=1
            || starts[0].navigation_sequence!=Some(sequence)||finishes[0].navigation_sequence!=Some(sequence)
            || starts[0].sequence>=finishes[0].sequence
            || settled.target.document_generation<=previous_generation||host_commands(&after)!=host_commands(&before)+1{
            return Err("Cross-document Back/Forward lacked an actual exact native Start→Finish lifecycle".into());
        }
        cross_document.push(json!({"direction":if back{"back"}else{"forward"},"path":path,"native_started":starts[0],"native_finished":finishes[0],
            "phase":load(&settled)?.phase,"lifecycle":settled.lifecycle,"new_document_generation":true,"main_http_delta":http_document_count(fixture)-count,
            "http_cache_outcome":"measured independently; native lifecycle is required even for cache restoration"}));
    }
    let history_stay=history_beforeunload_stay(runtime,&id,&view,fixture).await?;
    Ok(json!({"production_native_history":true,"frozen_provider_authority_and_user_coordinator":true,"user_history_blocked_during_agent_run":true,"user_ready_trusted_spa_setup":true,"same_document":same_document,"stale_pre_history_reload_wire_sequence_rejected":true,"explicit_reload_after_history_control_fences":true,"cross_document":cross_document,"beforeunload_history_stay":history_stay,
        "http_document_requests":fixture.snapshot().iter().filter(|receipt|receipt.destination=="document").map(|receipt|json!({"path":receipt.path,"method":receipt.method})).collect::<Vec<_>>(),
        "no_dom_url_or_empty_response_fabricated_completion":true}))
}

async fn history_beforeunload_stay(runtime:&Arc<BrowserResource>,id:&str,view:&tauri::Webview,fixture:&Fixture)->Result<Value,String>{
    super::native_fixture_click(view,"#history-leave").await?;
    if super::evaluate(view,"fixtureHistoryLeaveTrusted").await?!=true{return Err("History beforeunload arm lacks native trusted input".into());}
    let original=tab(runtime,id).await?;let original_nonce=super::evaluate(view,"fixtureDocumentNonce").await?;
    let before=report(runtime,id).await?;let count=http_document_count(fixture);
    // This case deliberately observes the CDP history command reply before
    // replying to the native dialog. Its COMMANDS receipt has already settled;
    // false-deferral behavior must work without a pending command fallback.
    tokio::time::timeout(Duration::from_secs(5),runtime.user_command(BrowserTabCommand::Back{target:original.target.clone()})).await
        .map_err(|_|"History command did not reply before user Stay; settled-COMMANDS path was not observed")?.map_err(error)?;
    let deadline=Instant::now()+Duration::from_secs(5);
    let dialog=loop{
        let current=tab(runtime,id).await?;
        if let Some(dialog)=current.script_dialog.filter(|dialog|dialog.kind==BrowserDialogKind::BeforeUnload){break dialog;}
        if Instant::now()>=deadline{return Err("Native history Back produced no beforeunload dialog".into());}
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let blocked=tab(runtime,id).await?;
    if blocked.target!=original.target||blocked.url!=original.url||blocked.lifecycle!=BrowserTabLifecycle::Ready||load(&blocked)?.phase!=BrowserNavigationPhase::Finished
        || load(&blocked)?.content_state!=BrowserContentState::CurrentDocument||load(&blocked)?.estimated_progress!=Some(100)
        || http_document_count(fixture)!=count{
        return Err("History beforeunload pending replaced the actual last successful document loading facts".into());
    }
    no_native_document_transition(&report(runtime,id).await?,&before)?;
    runtime.user_command(BrowserTabCommand::Dialog{target:dialog.target,request_id:dialog.request_id,accept:false,text:None}).await.map_err(error)?;
    let retained=tab(runtime,id).await?;let after=report(runtime,id).await?;
    if retained.target!=original.target||retained.url!=original.url||retained.script_dialog.is_some()
        || load(&retained)?.phase!=BrowserNavigationPhase::Finished||retained.lifecycle!=BrowserTabLifecycle::Ready
        || load(&retained)?.content_state!=BrowserContentState::CurrentDocument||load(&retained)?.estimated_progress!=Some(100)
        || super::evaluate(view,"fixtureDocumentNonce").await?!=original_nonce||http_document_count(fixture)!=count{
        return Err("History Stay lost the actual established document or released its target HTTP request".into());
    }
    no_native_document_transition(&after,&before)?;
    let rejected:Vec<_>=since(&after,&before).into_iter().filter(|event|event.kind==BrowserNavigationEventKind::Cancelled
        && event.cancellation_reason==Some(BrowserCancellationReason::NavigationRejected)&&event.dispatch_sequence.is_some()
        && event.navigation_sequence.is_none()&&event.document_generation.is_none()).collect();
    Ok(json!({"trusted_native_arm":true,"history_command_replied_before_user_stay":true,"native_beforeunload_false_reply_completed":true,"retained_document_phase":load(&retained)?.phase,
        "retained_document_lifecycle":retained.lifecycle,"same_document_generation_and_nonce":true,"main_http_delta":0,"no_native_started_or_fake_finished":true,
        "control_dispatch_rejections_observed":rejected}))
}

fn load(tab:&BrowserTabSnapshot)->Result<&BrowserLoadSummary,String>{tab.load.as_ref().ok_or("Native history load summary missing".into())}
async fn tab(runtime:&Arc<BrowserResource>,id:&str)->Result<BrowserTabSnapshot,String>{runtime.snapshot().await.map_err(error)?.runtime.ok_or("Native history runtime missing")?.tabs.into_iter().find(|tab|tab.target.tab_id==id).ok_or("Native history tab missing".into())}
async fn finished(runtime:&Arc<BrowserResource>,id:&str,url:&str)->Result<BrowserTabSnapshot,String>{
    let deadline=Instant::now()+Duration::from_secs(5);
    loop{let tab=tab(runtime,id).await?;
        if tab.url==url&&tab.lifecycle==BrowserTabLifecycle::Ready&&tab.load.as_ref().is_some_and(|load|
            load.phase==BrowserNavigationPhase::Finished&&load.content_state==BrowserContentState::CurrentDocument&&load.estimated_progress==Some(100)){return Ok(tab);}
        if Instant::now()>=deadline{return Err(format!("Native history did not preserve/reach real Finished document: url={},load={:?},lifecycle={:?}",tab.url,tab.load,tab.lifecycle));}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn report(runtime:&Arc<BrowserResource>,id:&str)->Result<BrowserNavigationReport,String>{runtime.navigation_diagnostics(tab(runtime,id).await?.target).await.map_err(error)}
fn since<'a>(after:&'a BrowserNavigationReport,before:&BrowserNavigationReport)->Vec<&'a BrowserNavigationEvent>{let sequence=before.trace.events.last().map_or(0,|event|event.sequence);after.trace.events.iter().filter(|event|event.sequence>sequence).collect()}
fn no_native_document_transition(after:&BrowserNavigationReport,before:&BrowserNavigationReport)->Result<(),String>{
    if since(after,before).iter().any(|event|matches!(event.kind,BrowserNavigationEventKind::Started|BrowserNavigationEventKind::Committed|BrowserNavigationEventKind::Finished)){
        return Err("Same-document history fabricated a native document lifecycle event".into());}
    Ok(())
}
fn host_commands(report:&BrowserNavigationReport)->u64{report.trace.counters.user_commands+report.trace.counters.agent_commands}
fn http_document_count(fixture:&Fixture)->usize{fixture.snapshot().iter().filter(|receipt|receipt.destination=="document").count()}
