//! Real WebView2 developer evaluation, distinct from native user input.
use nomifun_browser_platform::{runtime::*, workspace::BrowserWorkspaceService};
use serde_json::json;
use std::{sync::Arc, time::Duration};
pub(super) static EXECUTION_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(super) const HTML: &str = r#"<!doctype html><meta charset=utf-8><title>Developer evaluation fixture</title>
<h1 id=result>Before script</h1><button id=clicker>Native click</button><output id=clicks>0</output>
<script>window.pageOnly=42;let clicks=0;clicker.onclick=e=>{if(e.isTrusted)document.getElementById('clicks').textContent=String(++clicks);};</script>"#;

pub(super) async fn verify(app: &tauri::AppHandle, url: &str) -> Result<serde_json::Value, String> {
    let service = BrowserWorkspaceService::new(Arc::new(super::host::DesktopBrowserHost::new(app.clone())));
    let key = BrowserWorkspaceKey { user_id: "evaluation-fixture".into(), conversation_id: "evaluation-fixture".into() };
    let workspace = service.ensure(key.clone(), "fixture".into(), BrowserProfile::Ephemeral).await.map_err(|e|e.to_string())?;
    let result = async {
        workspace.user_command(BrowserTabCommand::Create { url:url.into() }).await.map_err(|e|e.to_string())?;
        let target = super::wait_workspace_page(&workspace, url).await?;
        workspace.set_surface(BrowserSurfaceBounds { x:20.,y:60.,width:1000.,height:600. }, true, Default::default()).await.map_err(|e|e.to_string())?;
        let run = workspace.begin_run().await.map_err(|e|e.to_string())?;
        let request = |expression: &str| BrowserEvaluation { target:target.clone(), expression:expression.into() };
        let old = workspace.observe(&run, None).await.map_err(|e|e.to_string())?;
        let button = old.elements.iter().find(|element|element.name=="Native click").ok_or("Native button not observed")?.reference.clone();
        let evaluated = workspace.evaluate(&run, request("document.getElementById('result').textContent='开发者脚本'; ({text:document.getElementById('result').textContent,pageGlobal:typeof pageOnly})")).await.map_err(|e|format!("Initial developer evaluation: {e}"))?;
        match evaluated.outcome {
            BrowserEvaluationOutcome::Completed { value } if value==json!({"text":"开发者脚本","pageGlobal":"undefined"}) => {},
            other => return Err(format!("Unexpected evaluation result: {other:?}")),
        }
        if evaluated.execution_kind!="developer_script" {return Err("Script falsely reported browser input".into());}
        if workspace.act(&run, BrowserAction::Click { element:button, button:BrowserMouseButton::Left,click_count:1 }).await.err()!=Some(WorkspaceError::StaleObservation) {return Err("Evaluation did not invalidate old input references".into());}
        let fresh = workspace.observe(&run,None).await.map_err(|e|e.to_string())?;
        if !fresh.content.contains("开发者脚本") {return Err("Script changed a different page".into());}
        for expression in ["throw new Error('script fixture')", "Promise.resolve(1)", "'x'.repeat(131073)", "while(true){}"] {
            let evaluated = tokio::time::timeout(Duration::from_secs(12), workspace.evaluate(&run, request(expression))).await.map_err(|_|"Browser evaluation did not honor its execution bound")?.map_err(|e|format!("Evaluation of {expression}: {e}"))?;
            if !matches!(evaluated.outcome, BrowserEvaluationOutcome::ScriptError { .. }) {return Err(format!("Expected script failure: {evaluated:?}"));}
        }
        // The timed-out script must not leave termination armed for the next call.
        if !matches!(workspace.evaluate(&run,request("6*7")).await.map_err(|e|e.to_string())?.outcome, BrowserEvaluationOutcome::Completed {value} if value==42) {return Err("Page did not recover after bounded evaluation".into());}
        let pending = workspace.evaluate(&run,request("confirm('Developer dialog'); 42")).await.map_err(|e|format!("Dialog evaluation: {e}"))?;
        let dialog = match pending.outcome {BrowserEvaluationOutcome::AwaitingDialog {dialog}=>dialog,other=>return Err(format!("Missing evaluation dialog: {other:?}"))};
        let replied = workspace.respond_dialog(&run,BrowserDialogReply{target:dialog.target,request_id:dialog.request_id,accept:false,text:None}).await.map_err(|e|e.to_string())?;
        if !matches!(replied.outcome, BrowserActionOutcome::EvaluationResult {evaluation} if matches!(&evaluation.outcome,BrowserEvaluationOutcome::Completed{value} if *value==42)) {return Err("Dialog lost the retained evaluation result".into());}
        let mut stale = request("document.title='MUST NOT RUN'");
        stale.target.document_generation += 1;
        if workspace.evaluate(&run,stale).await.err()!=Some(WorkspaceError::StaleTarget) {return Err("Stale target executed developer code".into());}
        workspace.finish_run(&run).await.map_err(|e|e.to_string())?;
        if workspace.evaluate(&run,request("document.title='MUST NOT RUN'")).await.is_ok() {return Err("Finished run executed developer code".into());}
        let stopping = workspace.begin_run().await.map_err(|e|e.to_string())?;
        let executing = request("const probe=new XMLHttpRequest(); probe.open('GET','/evaluation-start',false); probe.send(); while(true){}");
        let worker = workspace.clone(); let worker_run = stopping.clone();
        let pending = tokio::spawn(async move { worker.evaluate(&worker_run, executing).await });
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if EXECUTION_STARTED.load(std::sync::atomic::Ordering::SeqCst) { break; }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }).await.map_err(|_|"No positive evidence that the cancellable script started")?;
        stopping.cancel();
        tokio::time::timeout(Duration::from_secs(12), workspace.finish_run(&stopping)).await.map_err(|_|"Stop did not settle the owned evaluation")?.map_err(|e|e.to_string())?;
        if pending.await.map_err(|e|e.to_string())?.is_ok() {return Err("Canceled evaluation reported success".into());}
        let after_stop = workspace.begin_run().await.map_err(|e|e.to_string())?;
        if !matches!(workspace.evaluate(&after_stop,request("21*2")).await.map_err(|e|e.to_string())?.outcome,BrowserEvaluationOutcome::Completed{value} if value==42) {return Err("Stopped evaluation poisoned the next turn".into());}
        workspace.finish_run(&after_stop).await.map_err(|e|e.to_string())?;
        Ok(json!({"same_native_dom":true,"isolated_from_page_and_semantic_globals":true,"explicit_script_fidelity":true,"old_references_invalidated":true,"bounded_errors_and_recovery":true,"dialog_result_preserved":true,"stale_and_finished_rejected":true,"stop_settles_owned_script_and_next_turn_runs":true}))
    }.await;
    service.close(&key).await.map_err(|e|e.to_string())?;
    result
}
