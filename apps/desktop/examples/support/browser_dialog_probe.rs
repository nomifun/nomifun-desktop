//! Exercises the native host dialog owner, not a product testing feature.
use super::windows::script_dialogs as dialogs;

pub(super) async fn verify(view: &tauri::Webview) -> Result<serde_json::Value, String> {
    // add_child returns before its initial HTTP navigation has necessarily
    // committed. Never mistake the initial about:blank for our fixture document.
    let initial_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if matches!(super::evaluate(view, "location.protocol==='http:' && document.readyState==='complete'").await, Ok(value) if value==true)
        {
            break;
        }
        if tokio::time::Instant::now() >= initial_deadline {
            return Err("Dialog fixture initial navigation did not finish".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let metadata = std::sync::Arc::new(std::sync::Mutex::new(
        nomifun_browser_platform::runtime::BrowserTabSnapshot {
            target: nomifun_browser_platform::runtime::BrowserTabTarget {
                tab_id: view.label().into(),
                runtime_generation: 1,
                document_generation: 1,
            },
            title: String::new(),
            url: String::new(),
            lifecycle: nomifun_browser_platform::runtime::BrowserTabLifecycle::Ready,
            can_go_back: false,
            can_go_forward: false,
            blocked_permissions: vec![],
            permission_requests: vec![],
            script_dialog: None,
            diagnostics: Default::default(),
        },
    ));
    let mut events = dialogs::install(view, metadata.clone(), Default::default())
        .await
        .map_err(|e| e.to_string())?;
    let result=async {
        // WebView2 consumes this setting when loading the next HTML document.
        super::evaluate(view,"window.dialogProbeOldDocument=true;true").await?;
        super::windows::protocol_call(view,"Page.reload",serde_json::json!({})).await?;
        let deadline=tokio::time::Instant::now()+std::time::Duration::from_secs(5);
        loop {
            if matches!(super::evaluate(view,"!window.dialogProbeOldDocument && document.readyState==='complete'").await,Ok(value) if value==true) {break;}
            if tokio::time::Instant::now()>=deadline {return Err("Dialog policy reload did not complete".into());}
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let mut outcomes=vec![];
        for (name,expression,accept,expected) in [
            ("confirm_accept","confirm('Native fixture confirmation')",true,serde_json::json!(true)),
            ("confirm_cancel","confirm('Native fixture confirmation')",false,serde_json::json!(false)),
            ("prompt_accept","prompt('Native fixture prompt','default')",true,serde_json::json!("native reply")),
            ("prompt_cancel","prompt('Native fixture prompt','default')",false,serde_json::Value::Null),
            ("prompt_default","prompt('Native fixture default prompt','默认值')",true,serde_json::json!("默认值")),
            ("prompt_long_default","prompt('界'.repeat(2000),'值'.repeat(2000))",true,serde_json::json!("值".repeat(2000))),
            ("host_cancel","confirm('Native fixture host cancellation')",false,serde_json::json!(false)),
            ("alert","(alert('Native fixture alert'),'continued')",true,serde_json::json!("continued")),
        ] {
        super::evaluate(view,&format!("document.body.innerHTML='<button id=dialog-probe>Native dialog probe</button>';document.getElementById('dialog-probe').onclick=()=>{{window.dialogProbeStarted=true;window.dialogProbeResult={expression}}};true")).await?;
        super::windows::set_user_input_enabled(view,false).await?;
        dialogs::resume(view).await.map_err(|e|e.to_string())?;
        let mut driver=super::automation::TabAutomation::default();
        let cancel=tokio_util::sync::CancellationToken::new();
        driver.activate_for_agent(view,&cancel).await.map_err(|e|e.to_string())?;
        let element=super::frame_element(&mut driver,view,"Native dialog probe").await?;
        let child=view.clone();
        let action_cancel=cancel.clone();
        let mut action=tokio::spawn(async move {
            let result=driver.act(&child,nomifun_browser_platform::runtime::BrowserAction::click(element),&action_cancel).await;
            (driver,result)
        });
        let opened=tokio::time::timeout(std::time::Duration::from_secs(3),async {
            loop {
                let dialog = events.borrow_and_update().clone();
                if let Some(dialog) = dialog { break Ok::<_, String>(dialog); }
                events.changed().await.map_err(|e| e.to_string())?;
            }
        }).await;
        if !matches!(opened,Ok(Ok(_))) {
            cancel.cancel();
            dialogs::cancel(view).await.map_err(|e| e.to_string())?;
            let (mut driver,completed)=action.await.map_err(|e|e.to_string())?;
            let _=driver.settle_agent(view).await;
            return Err(format!("Native script dialog event did not arrive; input={completed:?}"));
        }
        let dialog = opened.unwrap().unwrap();
        if dialog.message.is_empty() || dialog.origin == "null" { return Err(format!("Native fixture dialog metadata was lost: {dialog:?}")); }
        if name == "prompt_long_default" && (!dialog.text_truncated || dialog.message.len() > 4096 || dialog.default_text.len() > 4096) {
            return Err("Native dialog page text was not bounded".into());
        }
        let saved_target = metadata.lock().unwrap().target.clone();
        metadata.lock().unwrap().target.document_generation += 1;
        let invalidated = dialogs::respond(view, dialog.target.clone(), dialog.request_id.clone(), false, None).await;
        metadata.lock().unwrap().target = saved_target;
        if invalidated != Err(nomifun_browser_platform::runtime::WorkspaceError::StaleTarget) {
            return Err("Native dialog response ignored current document invalidation".into());
        }
        let mut stale_target = dialog.target.clone();
        stale_target.document_generation += 1;
        for (target, id) in [(stale_target, dialog.request_id.clone()), (dialog.target.clone(), "wrong-dialog".into())] {
            if dialogs::respond(view, target, id, false, None).await != Err(nomifun_browser_platform::runtime::WorkspaceError::StaleTarget) {
                return Err("Stale native dialog reply was not rejected".into());
            }
        }
        // This bounded observation waits on the actual input owner; it never
        // cancels or detaches it just because the dialog keeps it pending.
        let before=tokio::time::timeout(std::time::Duration::from_millis(150),&mut action).await;
        let completed_before_reply=before.is_ok();
        if name == "host_cancel" {
            dialogs::cancel(view).await.map_err(|e| e.to_string())?;
        } else {
            dialogs::respond(view, dialog.target.clone(), dialog.request_id.clone(), accept,
                (name == "prompt_accept").then(|| "native reply".into())).await.map_err(|e|e.to_string())?;
        }
        if dialogs::respond(view, dialog.target, dialog.request_id, false, None).await != Err(nomifun_browser_platform::runtime::WorkspaceError::StaleTarget) {
            return Err("Replayed dialog response was accepted".into());
        };
        let completed=match before {Ok(result)=>result,Err(_)=>tokio::time::timeout(std::time::Duration::from_secs(5),action).await.map_err(|_|"Input did not settle after native dialog reply")?};
        let (mut driver,completed)=completed.map_err(|e|e.to_string())?;
        let settled=driver.settle_agent(view).await;
        completed.and(settled).map_err(|e|e.to_string())?;
        if super::evaluate(view,"dialogProbeResult").await?!=expected {return Err(format!("Native {name} reply did not reach JavaScript"));}
        outcomes.push(serde_json::json!({"case":name,"input_completed_before_reply":completed_before_reply,"result_verified":true,"stale_and_replay_rejected":true}));
        }
        // Destruction, unlike an observation timeout, proves that a paused
        // native command cannot continue. The same host close path owns both.
        let child = view.clone();
        let mut pending_command = tokio::spawn(async move {
            super::windows::protocol_call(&child, "Runtime.evaluate", serde_json::json!({"expression":"confirm('Native close while pending')","returnByValue":true})).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if events.borrow_and_update().is_some() { break Ok::<_,String>(()); }
                events.changed().await.map_err(|e| e.to_string())?;
            }
        }).await.map_err(|_|"Close fixture dialog did not open")??;
        super::windows::close_native_view(view).await?;
        let stopped = tokio::time::timeout(std::time::Duration::from_secs(3), &mut pending_command).await
            .map_err(|_|"Native close did not settle the pending command")?.map_err(|e|e.to_string())?;
        if stopped.is_ok() || events.borrow().is_some() { return Err("Native close retained a live dialog or reported the paused command as successful".into()); }
        Ok(serde_json::json!({"native_deferral":true,"cases":outcomes,"native_close_settled_pending_command":true}))
    }.await;
    // Failures keep native input locked until the enclosing runner closes it.
    // Successful verification already proved native destruction above.
    if result.is_err() {
        let _ = dialogs::cancel(view).await;
    }
    result
}
