//! Owned visible Chrome dialog conformance; never attaches a personal Profile.
use super::*;
use chromiumoxide::cdp::browser_protocol::page::{EnableParams, HandleJavaScriptDialogParams};
use std::time::Duration;

async fn evaluate(conn: &Connection, session: &str, script: &str) -> Value {
    let mut params = EvaluateParams::new(script);
    params.return_by_value = Some(true);
    let value = conn.send(session, &params).await.unwrap();
    assert!(
        value.get("exceptionDetails").is_none(),
        "fixture evaluation failed: {value}"
    );
    value["result"]["value"].clone()
}
async fn observe(
    browser: &AttachedBrowser,
    grant: &GrantedTab,
    cancel: &CancellationToken,
) -> Value {
    tokio::time::timeout(
        Duration::from_secs(5),
        browser.execute_granted(
            grant,
            Command::Observe {
                tab_id: grant.id().into(),
            },
            cancel,
        ),
    )
    .await
    .expect("observe blocked behind a modal")
    .unwrap()
}
async fn click(browser: &AttachedBrowser, grant: &GrantedTab, cancel: &CancellationToken) -> Value {
    let observed = observe(browser, grant, cancel).await;
    tokio::time::timeout(
        Duration::from_secs(5),
        browser.execute_granted(
            grant,
            Command::Click {
                tab_id: grant.id().into(),
                observation_id: observed["observation_id"].as_str().unwrap().into(),
                ref_id: reference(&observed, "Increment"),
            },
            cancel,
        ),
    )
    .await
    .unwrap_or_else(|error| {
        let pending=browser.pending(&grant.target_id);
        panic!("input did not return its native dialog: {error}; connected={}; pending={}; dialog={:?}; result={:?}",browser.is_connected(),pending.is_some(),pending.as_ref().and_then(|job|job.dialogs.snapshot()),pending.as_ref().and_then(|job|job.job.clone().now_or_never()));
    })
    .unwrap()
}
async fn reply(
    browser: &AttachedBrowser,
    grant: &GrantedTab,
    dialog: &Value,
    accept: bool,
    text: Option<&str>,
    cancel: &CancellationToken,
) -> Value {
    tokio::time::timeout(
        Duration::from_secs(5),
        browser.execute_granted(
            grant,
            Command::Dialog {
                tab_id: grant.id().into(),
                dialog_id: dialog["script_dialog"]["dialog_id"]
                    .as_str()
                    .unwrap()
                    .into(),
                accept,
                prompt_text: text.map(str::to_owned),
            },
            cancel,
        ),
    )
    .await
    .expect("dialog reply deadlocked behind its input")
    .unwrap()
}

#[tokio::test]
#[ignore = "explicit NOMIFUN_CHROME_BINARY; owned visible Chrome dialogs and stop"]
async fn real_dialogs_resume_exact_input_and_stop_preserves_existing_modal() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("dialog-profile");
    let launched = crate::launch::launch_chrome(
        &crate::launch::LaunchConfig {
            chrome_path: std::env::var_os("NOMIFUN_CHROME_BINARY")
                .expect("explicit fixture Chrome")
                .into(),
            user_data_dir: profile.clone(),
            headful: true,
        },
        false,
    )
    .await
    .unwrap();
    let (mut owner, diagnostics) = launched.connect().await.unwrap();
    let (url, server) = fixture().await;
    let browser = AttachedBrowser::connect_port_file(&profile.join("DevToolsActivePort"))
        .await
        .unwrap();
    let result=std::panic::AssertUnwindSafe(async {
        let target=diagnostics.send(ROOT_SESSION,&CreateTargetParams::new(url.clone())).await.unwrap()["targetId"].as_str().unwrap().to_owned();
        let mut attach=AttachToTargetParams::new(target.clone());attach.flatten=Some(true);
        let session=diagnostics.send(ROOT_SESSION,&attach).await.unwrap()["sessionId"].as_str().unwrap().to_owned();
        diagnostics.registry().register_session(session.clone(),"page");
        for _ in 0..40 {if evaluate(&diagnostics,&session,"!!document.getElementById('counter')").await==true {break;}tokio::time::sleep(Duration::from_millis(25)).await;}
        evaluate(&diagnostics,&session,"window.afterDialog=0;window.fixtureNonce=crypto.randomUUID();true").await;
        let nonce=evaluate(&diagnostics,&session,"fixtureNonce").await;
        let choices=browser.tabs_for_user().await.unwrap();
        let grant=browser.grant_tab(&choices.tabs.iter().find(|tab|tab.url==url).unwrap().choice_id).await.unwrap();
        let cancel=CancellationToken::new();
        for (kind,script,accept,text,expected) in [
            ("alert","alert('fixture alert');window.dialogResult='alert-done'",true,None,json!("alert-done")),
            ("confirm","window.dialogResult=confirm('fixture confirm')",false,None,json!(false)),
            ("prompt","window.dialogResult=prompt('fixture prompt','DEFAULT_PROMPT_PRIVATE_SENTINEL')",true,Some("确认输入"),json!("确认输入")),
        ] {
            evaluate(&diagnostics,&session,&format!("counter.onclick=()=>{{{script};afterDialog++}};true")).await;
            eprintln!("dialog fixture stage: {kind}");
            let dialog=click(&browser,&grant,&cancel).await;
            assert_eq!(dialog["script_dialog"]["kind"],kind);assert_eq!(dialog["script_dialog"]["owned"],true);assert_eq!(dialog["action_pending"],true);
            assert!(!dialog.to_string().contains("DEFAULT_PROMPT_PRIVATE_SENTINEL"));
            assert!(browser.pending(&grant.target_id).unwrap().job.clone().now_or_never().is_none(),"original Chrome input ACK must still be retained");
            assert_eq!(reply(&browser,&grant,&dialog,accept,text,&cancel).await["completed"],true);
            assert_eq!(evaluate(&diagnostics,&session,"dialogResult").await,expected);
        }
        assert_eq!(evaluate(&diagnostics,&session,"afterDialog").await,3);
        evaluate(&diagnostics,&session,"counter.onclick=()=>{window.firstAnswer=confirm('first');window.dialogResult=prompt('second');afterDialog++};true").await;
        eprintln!("dialog fixture stage: sequential");
        let first=click(&browser,&grant,&cancel).await;
        let second=reply(&browser,&grant,&first,false,None,&cancel).await;
        assert_eq!(second["script_dialog"]["kind"],"prompt");
        assert_ne!(first["script_dialog"]["dialog_id"],second["script_dialog"]["dialog_id"]);
        let old_retry=reply(&browser,&grant,&first,false,None,&cancel).await;
        assert_eq!(old_retry["script_dialog"]["dialog_id"],second["script_dialog"]["dialog_id"]);
        assert_eq!(reply(&browser,&grant,&second,true,Some("second answer"),&cancel).await["completed"],true);
        assert_eq!(evaluate(&diagnostics,&session,"firstAnswer===false&&dialogResult==='second answer'&&afterDialog===4").await,true);
        // The page's real cross-site iframe uses the same granted root Tool.
        let child_url=url.replace("127.0.0.1","localhost");
        evaluate(&diagnostics,&session,&format!("window.childLoaded=false;window.childResult=null;addEventListener('message',e=>{{if(e.data?.dialogFixture)childResult=e.data}});const child=document.createElement('iframe');child.id='dialogChild';child.style='position:absolute;left:20px;top:140px;width:440px;height:160px';child.onload=()=>childLoaded=true;child.src={};document.body.appendChild(child);true",serde_json::to_string(&child_url).unwrap())).await;
        for _ in 0..40 {if evaluate(&diagnostics,&session,"childLoaded").await==true {break;}tokio::time::sleep(Duration::from_millis(25)).await;}
        observe(&browser,&grant,&cancel).await;
        let (child,connection)={let state=browser.state.lock().unwrap();let automation=&state.automation[&grant.target_id];
            let sessions=automation.frames.routes.as_ref().unwrap().sessions();
            assert_eq!(sessions.len(),2,"fixture requires a real OOPIF session");
            (sessions.into_iter().find(|id|id!=&automation.session).unwrap(),state.connection.as_ref().unwrap().clone())};
        evaluate(&connection,&child,"counter.textContent='Child dialog';counter.onclick=e=>{const answer=prompt('child prompt');parent.postMessage({dialogFixture:true,answer,trusted:e.isTrusted},'*')};true").await;
        let observed=observe(&browser,&grant,&cancel).await;
        let child_dialog=tokio::time::timeout(Duration::from_secs(5),browser.execute_granted(&grant,Command::Click{tab_id:grant.id().into(),observation_id:observed["observation_id"].as_str().unwrap().into(),ref_id:reference(&observed,"Child dialog")},&cancel)).await.unwrap().unwrap();
        assert_eq!(child_dialog["script_dialog"]["kind"],"prompt");
        assert_eq!(reply(&browser,&grant,&child_dialog,true,Some("iframe answer"),&cancel).await["completed"],true);
        for _ in 0..40 {if evaluate(&diagnostics,&session,"childResult?.answer==='iframe answer'&&childResult.trusted").await==true {break;}tokio::time::sleep(Duration::from_millis(25)).await;}
        assert_eq!(evaluate(&diagnostics,&session,"childResult?.answer==='iframe answer'&&childResult.trusted").await,true);
        evaluate(&diagnostics,&session,"counter.onclick=()=>{window.dialogResult=confirm('cancel this action');afterDialog++};true").await;
        eprintln!("dialog fixture stage: stop confirm");
        let awaiting=click(&browser,&grant,&cancel).await;assert_eq!(awaiting["script_dialog"]["kind"],"confirm");
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5),browser.release_granted(&grant)).await.expect("Stop did not settle its modal input").unwrap();
        assert!(browser.is_connected());
        assert_eq!(evaluate(&diagnostics,&session,"dialogResult===false&&afterDialog===5&&trustedEvents.every(event=>event.trusted)").await,true);
        assert_eq!(evaluate(&diagnostics,&session,"fixtureNonce").await,nonce);
        let unload=CancellationToken::new();
        observe(&browser,&grant,&unload).await;
        evaluate(&diagnostics,&session,"window.onbeforeunload=e=>{e.preventDefault();e.returnValue=''};true").await;
        let leaving=tokio::time::timeout(Duration::from_secs(5),browser.execute_granted(&grant,Command::Navigate{tab_id:grant.id().into(),url:format!("{url}?leaving")},&unload)).await.unwrap().unwrap();
        assert_eq!(leaving["script_dialog"]["kind"],"beforeunload");
        unload.cancel();
        tokio::time::timeout(Duration::from_secs(5),browser.release_granted(&grant)).await.unwrap().unwrap();
        assert_eq!(evaluate(&diagnostics,&session,"fixtureNonce").await,nonce,"Stop must not accept navigation away from the existing page");
        evaluate(&diagnostics,&session,"window.onbeforeunload=null;true").await;
        let mut opening=diagnostics.subscribe("Page.javascriptDialogOpening",Some(&session));
        diagnostics.send(&session,&EnableParams::default()).await.unwrap();
        evaluate(&diagnostics,&session,"setTimeout(()=>{window.existingAnswer=confirm('existing user dialog')},0);true").await;
        tokio::time::timeout(Duration::from_secs(5),opening.recv()).await.unwrap().unwrap();
        let next=CancellationToken::new();
        let mut existing=Box::pin(browser.execute_granted(&grant,Command::Observe{tab_id:grant.id().into()},&next));
        let initial=tokio::time::timeout(Duration::from_millis(200),&mut existing).await;
        if let Ok(Ok(value))=&initial {
            assert_eq!(value["script_dialog"]["owned"],false);assert_eq!(value["action_pending"],false);
        }
        next.cancel();
        if initial.is_err() {
            assert!(tokio::time::timeout(Duration::from_secs(5),&mut existing).await.expect("Stop must cancel a read blocked by an unreported existing dialog").is_err());
        }
        tokio::time::timeout(Duration::from_secs(5),browser.release_granted(&grant)).await.unwrap().unwrap();
        diagnostics.send(&session,&HandleJavaScriptDialogParams::new(false)).await.expect("NomiFun must leave the user's original modal open");
        assert_eq!(evaluate(&diagnostics,&session,"existingAnswer").await,false);
        assert_eq!(evaluate(&diagnostics,&session,"fixtureNonce").await,nonce);
        assert!(owner.child_mut().try_wait().unwrap().is_none());
    }).catch_unwind().await;
    let disconnected = browser.disconnect().await;
    diagnostics.shutdown().await;
    let closed = owner.shutdown().await;
    drop(browser);
    drop(diagnostics);
    drop(owner);
    drop(server);
    let removed = directory.close();
    assert!(disconnected.is_ok() && closed.is_ok() && removed.is_ok());
    result.unwrap();
    println!(
        "SYSTEM_DIALOG_NATIVE_PASS alert=true confirm=true prompt=true sequential=true oopif=true stale_reply_no_replay=true stop_rejects_confirm=true beforeunload_preserves_document=true existing_modal_preserved=true temporary_cleanup=true"
    );
}
