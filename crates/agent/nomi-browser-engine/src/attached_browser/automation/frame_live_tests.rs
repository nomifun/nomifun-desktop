//! Visible, disposable Chrome only. Fixture JS observes native input and sets
//! up navigation/occlusion; it never creates or dispatches input events.
use super::*;
use chromiumoxide::cdp::browser_protocol::dom_debugger::GetEventListenersParams;
use chromiumoxide::cdp::browser_protocol::target::{GetTargetInfoParams, GetTargetsParams};
use chromiumoxide::cdp::js_protocol::runtime::EnableParams as EnableRuntimeParams;
use std::time::Duration;

const PRIVATE_SENTINEL: &str = "UNGRANTED_FRAME_FIXTURE_PRIVATE_PAGE";
const ROOT_HTML: &str = r#"<!doctype html><meta charset="utf-8"><title>Granted frame fixture</title>
<style>body{margin:12px;font:14px sans-serif}.frames{display:flex;gap:12px;margin-top:12px}iframe{width:320px;height:400px;border:3px solid #999}</style>
<button>Root action</button><div class="frames"><div id="same-host"></div><iframe id="foreign" src="http://localhost:__PORT__/foreign"></iframe></div>
<script>
window.fixtureNonce=crypto.randomUUID();window.receipts={};
addEventListener('message',event=>{
  if(!['http://127.0.0.1:__PORT__','http://localhost:__PORT__'].includes(event.origin))return;
  if(event.data?.tag==='native-frame-proof'&&['same','nested','foreign'].includes(event.data.scope))receipts[event.data.scope]=event.data;
});
document.getElementById('same-host').attachShadow({mode:'open'}).innerHTML='<style>iframe{width:320px;height:400px;border:3px solid #999}</style><iframe id="same" src="/same"></iframe>';
</script>"#;
const CHILD_HTML: &str = r#"<!doctype html><meta charset="utf-8"><title>__LABEL__ fixture</title>
<style>body{margin:8px;font:14px sans-serif;height:1800px}input{width:130px}button{margin:6px 0}iframe{display:block;width:272px;height:245px;border:3px solid #678;margin-top:8px}</style>
<input id="field" aria-label="__LABEL__ field"><button id="action">__LABEL__ action</button>__NESTED__
<script>
window.clicks=0;window.events=[];window.frameNonce=crypto.randomUUID();
const scope='__SCOPE__';const epoch=new URLSearchParams(location.search).get('epoch')||'initial';
function report(){top.postMessage({tag:'native-frame-proof',scope,epoch,nonce:frameNonce,clicks,value:field.value,scroll:scrollY,events},'http://127.0.0.1:__PORT__')}
for(const type of ['pointerdown','pointerup','click','input','keydown','keyup','wheel'])document.addEventListener(type,e=>{
  if(events.length<128)events.push({type:e.type,trusted:e.isTrusted,target:e.target.id||'',key:e.key||''});
  queueMicrotask(report);
},true);
action.onclick=()=>{clicks++;report()};addEventListener('scroll',report);addEventListener('load',report);
</script>"#;

async fn frame_fixture() -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let worker = tokio::spawn(async move {
        let mut requests = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                Some(_)=requests.join_next(), if !requests.is_empty()=>{},
                accepted=listener.accept(), if requests.len()<16=>{
                    let Ok((mut stream,_))=accepted else {break;};
                    requests.spawn(async move {
                        let mut request=[0u8;4096];
                        let size=match tokio::time::timeout(Duration::from_secs(2),stream.read(&mut request)).await {
                            Ok(Ok(size)) if size>0=>size,_=>return,
                        };
                        let path=std::str::from_utf8(&request[..size]).ok().and_then(|line|line.split_whitespace().nth(1))
                            .unwrap_or("/").split('?').next().unwrap_or("/");
                        let body=match path {
                            "/root"=>ROOT_HTML.to_owned(),
                            "/same"=>CHILD_HTML.replace("__LABEL__","Same parent").replace("__SCOPE__","same")
                                .replace("__NESTED__","<iframe id='nested' src='/nested'></iframe>"),
                            "/nested"=>CHILD_HTML.replace("__LABEL__","Same nested").replace("__SCOPE__","nested").replace("__NESTED__",""),
                            "/foreign"=>CHILD_HTML.replace("__LABEL__","Cross site").replace("__SCOPE__","foreign").replace("__NESTED__",""),
                            "/private"=>format!("<!doctype html><title>{PRIVATE_SENTINEL}</title><button>{PRIVATE_SENTINEL}</button>"),
                            _=>"<!doctype html>".into(),
                        }.replace("__PORT__",&port.to_string());
                        let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                        let _=stream.write_all(response.as_bytes()).await;
                    });
                }
            }
        }
    });
    (format!("http://127.0.0.1:{port}/root"), Server(worker))
}

async fn read_root(conn: &Connection, session: &str, script: &str) -> Value {
    let mut params = EvaluateParams::new(script);
    params.return_by_value = Some(true);
    let result = conn.send(session, &params).await.unwrap();
    assert!(
        result.get("exceptionDetails").is_none(),
        "fixture observation failed: {result}"
    );
    result["result"]["value"].clone()
}

async fn wait_root(conn: &Connection, session: &str, script: &str) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while read_root(conn, session, script).await != true {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("real fixture document/event witness did not arrive");
}

async fn observe_all(
    browser: &AttachedBrowser,
    grant: &GrantedTab,
    cancel: &CancellationToken,
) -> Value {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let result = browser
                .execute_granted(
                    grant,
                    Command::Observe {
                        tab_id: grant.id().into(),
                    },
                    cancel,
                )
                .await;
            match result {
                Ok(value) => {
                    let names: Vec<_> = value["elements"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .filter_map(|element| element["name"].as_str())
                        .collect();
                    if [
                        "Root action",
                        "Same parent field",
                        "Same nested field",
                        "Cross site field",
                    ]
                    .iter()
                    .all(|name| names.contains(name))
                    {
                        assert_eq!(value["coverage"], "page_frames");
                        assert_eq!(value["unobserved_child_frames"], 0);
                        assert!(!value.to_string().contains(PRIVATE_SENTINEL));
                        let ids: Vec<_> = value["elements"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|element| element["ref_id"].as_str().unwrap())
                            .collect();
                        assert_eq!(
                            ids.len(),
                            ids.iter().collect::<BTreeSet<_>>().len(),
                            "frame-local refs collided"
                        );
                        return value;
                    }
                }
                Err(Error::ExecutionFailed | Error::InvalidInput) => {}
                Err(error) => panic!("fixture observation failed: {error}"),
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("Observe never covered root, same-process nested frame, and OOPIF")
}

/// CDP observer only: inspect the existing semantic Window objects; do not
/// monkeypatch EventTarget or create replacement contexts. MutationObserver
/// registrations are not exposed by getEventListeners and are not claimed by
/// this witness. Context-created events detect accidental world proliferation.
async fn repeated_observe_keeps_worlds_and_window_listeners_stable(
    browser: &AttachedBrowser,
    grant: &GrantedTab,
    cancel: &CancellationToken,
) {
    observe_all(browser, grant, cancel).await;
    let (sessions, connection) = {
        let _operation = browser.operations.lock().await;
        let connection = browser.current_connection().unwrap();
        let state = browser.state.lock().unwrap();
        let automation = &state.automation[&grant.target_id];
        (
            automation.frames.routes.as_ref().unwrap().sessions(),
            connection,
        )
    };
    let mut receivers = Vec::new();
    let mut worlds = Vec::new();
    for session in &sessions {
        let mut receiver = connection.subscribe("Runtime.executionContextCreated", Some(session));
        connection
            .send(session, &EnableRuntimeParams::default())
            .await
            .unwrap();
        loop {
            match receiver.try_recv() {
                Ok(event) => {
                    let context = &event.params["context"];
                    if context["name"] == "nomifun-system-browser-semantic" {
                        worlds.push((
                            session.clone(),
                            context["id"].as_i64().unwrap(),
                            context["uniqueId"].as_str().unwrap().to_owned(),
                        ));
                    }
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                Err(error) => panic!("lost context lifecycle evidence: {error}"),
            }
        }
        receivers.push(receiver);
    }
    assert_eq!(
        worlds.len(),
        4,
        "must inspect root, same, nested, and OOPIF worlds"
    );
    assert_eq!(
        worlds
            .iter()
            .map(|world| &world.2)
            .collect::<BTreeSet<_>>()
            .len(),
        4
    );
    let before = window_listener_counts(&connection, &worlds).await;
    for (session, context, _) in &worlds {
        let group = format!("fixture-semantic-gc-{}", nomifun_common::generate_id());
        let mut evaluate = EvaluateParams::new(native_semantic::initialization_expression());
        evaluate.context_id = Some(ExecutionContextId::new(*context));
        evaluate.object_group = Some(group.clone());
        let value = connection.send(session, &evaluate).await.unwrap();
        let object = value["result"]["objectId"]
            .as_str()
            .expect("probe semantic core");
        let mut weak = CallFunctionOnParams::new(
            "function(){globalThis.__nomiFixtureCoreWeakRef=new WeakRef(this);return globalThis.__nomiFixtureCoreWeakRef.deref()===this;}",
        );
        weak.object_id = Some(RemoteObjectId::new(object));
        weak.return_by_value = Some(true);
        assert_eq!(
            connection.send(session, &weak).await.unwrap()["result"]["value"],
            true
        );
        connection
            .send(session, &ReleaseObjectGroupParams::new(group))
            .await
            .unwrap();
        connection
            .send(
                session,
                &chromiumoxide::cdp::js_protocol::heap_profiler::CollectGarbageParams::default(),
            )
            .await
            .unwrap();
        let mut inspect = EvaluateParams::new(
            "(()=>{const gone=globalThis.__nomiFixtureCoreWeakRef.deref()===undefined;delete globalThis.__nomiFixtureCoreWeakRef;return gone;})()",
        );
        inspect.context_id = Some(ExecutionContextId::new(*context));
        inspect.return_by_value = Some(true);
        assert_eq!(
            connection.send(session, &inspect).await.unwrap()["result"]["value"],
            true,
            "semantic core remained reachable after its object group was released"
        );
    }
    for _ in 0..5 {
        observe_all(browser, grant, cancel).await;
        assert_eq!(
            window_listener_counts(&connection, &worlds).await,
            before,
            "repeated Observe added persistent Window listeners"
        );
        for receiver in &mut receivers {
            loop {
                match receiver.try_recv() {
                    Ok(event) => assert_ne!(
                        event.params["context"]["name"], "nomifun-system-browser-semantic",
                        "Observe allocated a new semantic context in an unchanged document"
                    ),
                    Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                    Err(error) => panic!("lost context lifecycle evidence: {error}"),
                }
            }
        }
    }
}

async fn window_listener_counts(
    connection: &Connection,
    worlds: &[(String, i64, String)],
) -> Vec<usize> {
    let mut counts = Vec::new();
    for (session, context, _) in worlds {
        let group = format!("fixture-window-listeners-{}", nomifun_common::generate_id());
        let mut query = EvaluateParams::new("globalThis");
        query.context_id = Some(ExecutionContextId::new(*context));
        query.object_group = Some(group.clone());
        let window = connection.send(session, &query).await.unwrap();
        assert!(window.get("exceptionDetails").is_none());
        let object = window["result"]["objectId"]
            .as_str()
            .expect("existing semantic Window");
        let result = connection
            .send(session, &GetEventListenersParams::new(object.to_owned()))
            .await;
        connection
            .send(session, &ReleaseObjectGroupParams::new(group))
            .await
            .unwrap();
        counts.push(result.unwrap()["listeners"].as_array().unwrap().len());
    }
    counts
}

#[tokio::test]
#[ignore = "explicit NOMIFUN_CHROME_BINARY; visible owned Chrome/Profile, native iframe input only"]
async fn real_granted_frames_have_trusted_input_and_reject_occluded_or_replaced_refs() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("frame-profile");
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
    let (url, server) = frame_fixture().await;
    let mut attached = None;
    let mut granted = None;
    let result=std::panic::AssertUnwindSafe(async {
        let target=diagnostics.send(ROOT_SESSION,&CreateTargetParams::new(url.clone())).await.unwrap()["targetId"].as_str().unwrap().to_owned();
        let private_url=url.replace("/root","/private");
        let private_target=diagnostics.send(ROOT_SESSION,&CreateTargetParams::new(private_url.clone())).await.unwrap()["targetId"].as_str().unwrap().to_owned();
        let mut params=AttachToTargetParams::new(target.clone());params.flatten=Some(true);
        let session=diagnostics.send(ROOT_SESSION,&params).await.unwrap()["sessionId"].as_str().unwrap().to_owned();
        diagnostics.registry().register_session(session.clone(),"page");
        wait_root(&diagnostics,&session,"!!window.receipts?.same && !!receipts.nested && !!receipts.foreign").await;
        let nonce=read_root(&diagnostics,&session,"fixtureNonce").await;
        let targets=diagnostics.send(ROOT_SESSION,&GetTargetsParams::default()).await.unwrap();
        assert!(targets["targetInfos"].as_array().unwrap().iter().any(|target|target["type"]=="iframe"&&target["url"].as_str().is_some_and(|url|url.starts_with("http://localhost:")&&url.ends_with("/foreign"))),"cross-site fixture did not create a real OOPIF: {targets}");
        attached=Some(AttachedBrowser::connect_port_file(&profile.join("DevToolsActivePort")).await.unwrap());
        let browser=attached.as_ref().unwrap();
        let choices=browser.tabs_for_user().await.unwrap();
        assert!(choices.tabs.iter().any(|tab|tab.url==private_url),"unselected sentinel tab missing");
        let choice=choices.tabs.iter().find(|tab|tab.url==url).unwrap();
        granted=Some(browser.grant_tab(&choice.choice_id).await.unwrap());
        let grant=granted.as_ref().unwrap();
        let cancel=CancellationToken::new();
        repeated_observe_keeps_worlds_and_window_listeners_stable(browser,grant,&cancel).await;
        for (scope,label) in [("nested","Same nested"),("same","Same parent"),("foreign","Cross site")] {
            let observation=observe_all(browser,grant,&cancel).await;
            let observation_id=observation["observation_id"].as_str().unwrap().to_owned();
            let text=format!("中文输入-{scope}");
            browser.execute_granted(grant,Command::Type {tab_id:grant.id().into(),observation_id:observation_id.clone(),ref_id:reference(&observation,&format!("{label} field")),text:text.clone(),replace:true},&cancel).await.unwrap();
            if scope!="foreign" {
                let focused=if scope=="nested" {"same.contentDocument.activeElement.id==='nested' && same.contentDocument.getElementById('nested').contentDocument.activeElement.id==='field'"} else {"same.contentDocument.activeElement.id==='field'"};
                assert_eq!(read_root(&diagnostics,&session,&format!("(()=>{{const host=document.getElementById('same-host');const same=host.shadowRoot.getElementById('same');return document.activeElement===host && host.shadowRoot.activeElement===same && ({focused})}})()")).await,true,"native focus did not traverse the shadow-root iframe chain");
            }
            browser.execute_granted(grant,Command::Press {tab_id:grant.id().into(),observation_id:observation_id.clone(),keys:"End".into()},&cancel).await.unwrap();
            browser.execute_granted(grant,Command::Click {tab_id:grant.id().into(),observation_id:observation_id.clone(),ref_id:reference(&observation,&format!("{label} action"))},&cancel).await.unwrap();
            wait_root(&diagnostics,&session,&format!("receipts.{scope}.clicks===1 && receipts.{scope}.value==={}",serde_json::to_string(&text).unwrap())).await;
            if scope!="same" {
                browser.execute_granted(grant,Command::Scroll {tab_id:grant.id().into(),observation_id:observation_id.clone(),delta_x:0.0,delta_y:320.0},&cancel).await.unwrap();
                wait_root(&diagnostics,&session,&format!("receipts.{scope}.scroll>0 && receipts.{scope}.events.some(e=>e.type==='wheel'&&e.trusted)")).await;
                browser.execute_granted(grant,Command::Scroll {tab_id:grant.id().into(),observation_id:observation_id.clone(),delta_x:0.0,delta_y:-320.0},&cancel).await.unwrap();
                wait_root(&diagnostics,&session,&format!("receipts.{scope}.scroll===0")).await;
            }
            let proof=read_root(&diagnostics,&session,&format!("receipts.{scope}")).await;
            let events=proof["events"].as_array().unwrap();
            assert!(events.iter().all(|event|event["trusted"]==true),"synthetic frame input: {proof}");
            for kind in ["pointerdown","pointerup","input","keydown","keyup","click"] {
                assert!(events.iter().any(|event|event["type"]==kind),"missing {kind} in {scope}: {proof}");
            }
            assert!(events.iter().any(|event|event["type"]=="keydown"&&event["key"]=="End"));
        }
        let observation=observe_all(browser,grant,&cancel).await;
        let observation_id=observation["observation_id"].as_str().unwrap().to_owned();
        let old_foreign=reference(&observation,"Cross site action");
        read_root(&diagnostics,&session,"(()=>{const r=foreign.getBoundingClientRect();const cover=document.createElement('div');cover.id='cover';cover.style.cssText=`position:fixed;left:${r.x}px;top:${r.y}px;width:${r.width}px;height:${r.height}px;background:#ccc;z-index:10000`;document.body.append(cover);return true})()").await;
        assert_eq!(browser.execute_granted(grant,Command::Click {tab_id:grant.id().into(),observation_id:observation_id.clone(),ref_id:old_foreign},&cancel).await.err(),Some(Error::InvalidInput),"covered iframe must reject native input");
        read_root(&diagnostics,&session,"cover.remove();true").await;
        // Renew after the intentionally rejected occluded action, so the next
        // rejection proves child navigation rather than prior failure state.
        let observation=observe_all(browser,grant,&cancel).await;
        let observation_id=observation["observation_id"].as_str().unwrap().to_owned();
        let nested_nonce=read_root(&diagnostics,&session,"receipts.nested.nonce").await;
        let old_nested=reference(&observation,"Same nested action");
        read_root(&diagnostics,&session,"document.getElementById('same-host').shadowRoot.getElementById('same').contentDocument.getElementById('nested').src='/nested?epoch=replaced';true").await;
        wait_root(&diagnostics,&session,"receipts.nested.epoch==='replaced'").await;
        assert_ne!(read_root(&diagnostics,&session,"receipts.nested.nonce").await,nested_nonce);
        assert_eq!(browser.execute_granted(grant,Command::Click {tab_id:grant.id().into(),observation_id,ref_id:old_nested},&cancel).await.err(),Some(Error::InvalidInput),"old child-document reference must be revoked");
        let fresh=observe_all(browser,grant,&cancel).await;
        browser.execute_granted(grant,Command::Click {tab_id:grant.id().into(),observation_id:fresh["observation_id"].as_str().unwrap().into(),ref_id:reference(&fresh,"Same nested action")},&cancel).await.unwrap();
        wait_root(&diagnostics,&session,"receipts.nested.clicks===1").await;
        assert_eq!(read_root(&diagnostics,&session,"receipts.same.clicks===1 && receipts.foreign.clicks===1").await,true);
        browser.release_granted(grant).await.unwrap();
        browser.disconnect().await.unwrap();
        assert!(owner.child_mut().try_wait().unwrap().is_none(),"released instrumentation must not close user's browser");
        for id in [&target,&private_target] {
            let params=GetTargetInfoParams::builder().target_id(id.clone()).build();
            assert_eq!(diagnostics.send(ROOT_SESSION,&params).await.unwrap()["targetInfo"]["targetId"],*id);
        }
        assert_eq!(read_root(&diagnostics,&session,"fixtureNonce").await,nonce);
        assert_eq!(read_root(&diagnostics,&session,"receipts.nested.clicks===1 && receipts.same.clicks===1 && receipts.foreign.clicks===1").await,true);
    }).catch_unwind().await;
    // Even assertion panics first retire the borrowed socket and then the
    // independent fixture owner. The private extra page belongs only to it.
    let mut detached = Ok(());
    if let Some(browser) = attached.as_ref() {
        if let Some(grant) = granted.as_ref() {
            let _ = browser.release_granted(grant).await;
        }
        detached = browser.disconnect().await;
    }
    diagnostics.shutdown().await;
    let closed = owner.shutdown().await;
    drop(attached);
    drop(diagnostics);
    drop(owner);
    drop(server);
    let removed = directory.close();
    assert!(
        detached.is_ok() && closed.is_ok() && removed.is_ok(),
        "fixture cleanup: disconnect={detached:?}; browser={closed:?}; profile={removed:?}"
    );
    result.unwrap();
}
