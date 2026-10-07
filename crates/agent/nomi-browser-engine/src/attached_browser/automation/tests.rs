use super::*;
use chromiumoxide::cdp::browser_protocol::target::CreateTargetParams;
use futures_util::FutureExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[path = "cancellation_tests.rs"]
mod cancellation;

#[path = "initialization_tests.rs"]
mod initialization;

#[path = "dialog_tests.rs"]
mod dialogs;
#[path = "frame_live_tests.rs"]
mod frame_live;

struct Server(tokio::task::JoinHandle<()>);
impl Drop for Server {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn fixture() -> (String, Server) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/fixture", listener.local_addr().unwrap());
    let worker = tokio::spawn(async move {
        let mut requests = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                Some(_) = requests.join_next(), if !requests.is_empty()=>{},
                accepted=listener.accept(), if requests.len()<16=>{
                let Ok((mut stream,_))=accepted else {break;};
                requests.spawn(async move {
                let mut request = [0; 4096];
                if !matches!(tokio::time::timeout(std::time::Duration::from_secs(2),stream.read(&mut request)).await,Ok(Ok(size)) if size>0) {return;}
                let body = r#"<!doctype html><html><body style="height:4000px;margin:20px">
                <button id="counter">Increment</button><input id="nameInput" aria-label="Name"><div role="status" id="status">0</div>
                <button id="hoverTrap">Hover trap</button><input id="other" aria-label="Other"><input type="password" role="textbox" aria-label="Password" value="shortFixtureSecret">
                <script>window.trustedEvents=[];window.count=0;window.trapCount=0;window.armHoverTrap=false;window.armFocusTrap=false;
                for(const type of ['pointerdown','pointerup','mousedown','mouseup','click','input','keydown','keyup','wheel'])
                    document.addEventListener(type,e=>trustedEvents.push({type:e.type,trusted:e.isTrusted,key:e.key||'',value:e.target.value||''}),true);
                counter.onclick=()=>{status.textContent=String(++count);document.querySelector('[role=status]').textContent=String(count)};
                hoverTrap.onmousemove=()=>{if(armHoverTrap){const replacement=hoverTrap.cloneNode(true);replacement.id='replacedTrap';replacement.onclick=()=>trapCount++;hoverTrap.replaceWith(replacement);}};
                nameInput.onkeydown=e=>{if(armFocusTrap&&e.ctrlKey&&e.key.toLowerCase()==='a')other.focus()};
                </script></body></html>"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes()).await;
                });
            }}
        }
    });
    (url, Server(worker))
}

async fn inspect(browser: &AttachedBrowser, grant: &GrantedTab) -> Value {
    let value=fixture_eval(browser,grant,"JSON.stringify({count:window.count,value:nameInput.value,other:other.value,trap:window.trapCount,replaced:!!document.getElementById('replacedTrap'),scroll:scrollY,events:window.trustedEvents})").await;
    serde_json::from_str(value.as_str().unwrap()).unwrap()
}

async fn fixture_eval(browser: &AttachedBrowser, grant: &GrantedTab, script: &str) -> Value {
    let _operation = browser.operations.lock().await;
    let connection = browser.current_connection().unwrap();
    let session = browser.state.lock().unwrap().automation[&grant.target_id]
        .session
        .clone();
    let mut evaluate = EvaluateParams::new(script);
    evaluate.return_by_value = Some(true);
    let value = connection.send(&session, &evaluate).await.unwrap();
    value["result"]["value"].clone()
}

fn reference(observation: &Value, name: &str) -> String {
    observation["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["name"] == name)
        .unwrap_or_else(|| panic!("fixture element {name} missing: {observation}"))
        .get("ref_id")
        .unwrap()
        .as_str()
        .unwrap()
        .into()
}

#[tokio::test]
#[ignore = "explicit NOMIFUN_CHROME_BINARY; opens only an owned visible disposable fixture"]
async fn real_attached_tab_uses_trusted_input_and_disconnect_preserves_the_browser() {
    let executable =
        std::env::var_os("NOMIFUN_CHROME_BINARY").expect("explicit fixture browser executable");
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("fixture-profile");
    let launched = crate::launch::launch_chrome(
        &crate::launch::LaunchConfig {
            chrome_path: executable.into(),
            user_data_dir: profile.clone(),
            headful: true,
        },
        false,
    )
    .await
    .unwrap();
    let (mut owner, diagnostics) = launched.connect().await.unwrap();
    let (url, _server) = fixture().await;
    let result = std::panic::AssertUnwindSafe(async {
        diagnostics
            .send(ROOT_SESSION, &CreateTargetParams::new(url.clone()))
            .await
            .unwrap();
        let browser = AttachedBrowser::connect_port_file(&profile.join("DevToolsActivePort"))
            .await
            .unwrap();
        let tabs = browser.tabs_for_provider().await.unwrap();
        let tab = tabs
            .into_iter()
            .find(|tab| tab.info.url == url)
            .expect("owned fixture tab");
        let grant = tab.grant;
        let cancel = CancellationToken::new();
        let mut observation = Value::Null;
        for _ in 0..30 {
            observation = match browser
                .execute_granted(
                    &grant,
                    Command::Observe {
                        tab_id: grant.id().into(),
                    },
                    &cancel,
                )
                .await
            {
                Ok(observation) => observation,
                Err(Error::ExecutionFailed | Error::InvalidInput) => {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    continue;
                }
                Err(error) => panic!("fixture observe failed: {error}"),
            };
            if observation["elements"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["name"] == "Increment")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        let id = observation["observation_id"].as_str().unwrap().to_owned();
        assert!(
            !observation.to_string().contains("shortFixtureSecret"),
            "password values must not reach the Agent"
        );
        browser
            .execute_granted(
                &grant,
                Command::Click {
                    tab_id: grant.id().into(),
                    observation_id: id.clone(),
                    ref_id: reference(&observation, "Increment"),
                },
                &cancel,
            )
            .await
            .unwrap();
        browser
            .execute_granted(
                &grant,
                Command::Type {
                    tab_id: grant.id().into(),
                    observation_id: id.clone(),
                    ref_id: reference(&observation, "Name"),
                    text: "真实输入".into(),
                    replace: true,
                },
                &cancel,
            )
            .await
            .unwrap();
        browser
            .execute_granted(
                &grant,
                Command::Type {
                    tab_id: grant.id().into(),
                    observation_id: id.clone(),
                    ref_id: reference(&observation, "Name"),
                    text: String::new(),
                    replace: true,
                },
                &cancel,
            )
            .await
            .unwrap();
        assert_eq!(inspect(&browser, &grant).await["value"], "");
        browser
            .execute_granted(
                &grant,
                Command::Type {
                    tab_id: grant.id().into(),
                    observation_id: id.clone(),
                    ref_id: reference(&observation, "Name"),
                    text: "真实输入".into(),
                    replace: true,
                },
                &cancel,
            )
            .await
            .unwrap();
        fixture_eval(
            &browser,
            &grant,
            "window.armFocusTrap=true;window.armHoverTrap=true",
        )
        .await;
        assert_eq!(
            browser
                .execute_granted(
                    &grant,
                    Command::Type {
                        tab_id: grant.id().into(),
                        observation_id: id.clone(),
                        ref_id: reference(&observation, "Name"),
                        text: "must-not-enter-other-input".into(),
                        replace: true
                    },
                    &cancel
                )
                .await
                .err(),
            Some(Error::InvalidInput)
        );
        assert_eq!(inspect(&browser, &grant).await["other"], "");
        fixture_eval(&browser, &grant, "window.armFocusTrap=false").await;
        assert_eq!(
            browser
                .execute_granted(
                    &grant,
                    Command::Click {
                        tab_id: grant.id().into(),
                        observation_id: id.clone(),
                        ref_id: reference(&observation, "Hover trap")
                    },
                    &cancel
                )
                .await
                .err(),
            Some(Error::InvalidInput)
        );
        let guarded = inspect(&browser, &grant).await;
        assert_eq!(guarded["replaced"], true);
        assert_eq!(guarded["trap"], 0);
        browser
            .execute_granted(
                &grant,
                Command::Press {
                    tab_id: grant.id().into(),
                    observation_id: id.clone(),
                    keys: "End".into(),
                },
                &cancel,
            )
            .await
            .unwrap();
        let scrolled = browser
            .execute_granted(
                &grant,
                Command::Scroll {
                    tab_id: grant.id().into(),
                    observation_id: id.clone(),
                    delta_x: 0.0,
                    delta_y: 500.0,
                },
                &cancel,
            )
            .await;
        if let Err(error) = scrolled {
            let state = fixture_eval(&browser, &grant, "({visible:document.visibilityState,focused:document.hasFocus(),width:innerWidth,height:innerHeight,scroll:scrollY,events:trustedEvents.map(e=>e.type)})").await;
            panic!("scroll failed: {error}; fixture={state}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        let proof = inspect(&browser, &grant).await;
        assert_eq!(proof["count"], 1);
        assert_eq!(proof["value"], "真实输入");
        assert!(proof["scroll"].as_f64().unwrap() > 0.0);
        for kind in [
            "pointerdown",
            "pointerup",
            "click",
            "input",
            "keydown",
            "keyup",
            "wheel",
        ] {
            assert!(
                proof["events"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|event| event["type"] == kind && event["trusted"] == true),
                "missing trusted {kind}: {proof}"
            );
        }
        let fresh = browser
            .execute_granted(
                &grant,
                Command::Observe {
                    tab_id: grant.id().into(),
                },
                &cancel,
            )
            .await
            .unwrap();
        assert_ne!(fresh["observation_id"], id);
        assert_eq!(
            browser
                .execute_granted(
                    &grant,
                    Command::Press {
                        tab_id: grant.id().into(),
                        observation_id: id,
                        keys: "Enter".into()
                    },
                    &cancel
                )
                .await
                .err(),
            Some(Error::InvalidInput)
        );
        let other_browser = AttachedBrowser::connect_port_file(&profile.join("DevToolsActivePort"))
            .await
            .unwrap();
        let other_grant = other_browser
            .tabs_for_provider()
            .await
            .unwrap()
            .into_iter()
            .find(|tab| tab.info.url == url)
            .map(|tab| tab.grant)
            .unwrap();
        assert_eq!(
            grant.target_key(),
            other_grant.target_key(),
            "same real tab must contend across independent connections"
        );
        assert!(
            !grant.same_target(&other_grant),
            "per-connection grant identities are not interchangeable"
        );
        other_browser.disconnect().await.unwrap();
        let stale = fresh["observation_id"].as_str().unwrap().to_owned();
        browser
            .execute_granted(
                &grant,
                Command::Navigate {
                    tab_id: grant.id().into(),
                    url: format!("{url}?next"),
                },
                &cancel,
            )
            .await
            .unwrap();
        assert_eq!(
            browser
                .execute_granted(
                    &grant,
                    Command::Press {
                        tab_id: grant.id().into(),
                        observation_id: stale,
                        keys: "Enter".into()
                    },
                    &cancel
                )
                .await
                .err(),
            Some(Error::InvalidInput)
        );
        browser.release_granted(&grant).await.unwrap();
        browser.disconnect().await.unwrap();
        assert!(
            owner.child_mut().try_wait().unwrap().is_none(),
            "attaching host must not own/close the browser"
        );
        diagnostics
            .send(
                ROOT_SESSION,
                &chromiumoxide::cdp::browser_protocol::browser::GetVersionParams::default(),
            )
            .await
            .unwrap();
        Result::<(), String>::Ok(())
    })
    .catch_unwind()
    .await;
    diagnostics.shutdown().await;
    owner.shutdown().await.unwrap();
    result.unwrap().unwrap();
}

#[test]
fn page_input_rejects_browser_chrome_shortcuts_and_privileged_documents() {
    for keys in [
        "Ctrl+Tab",
        "Ctrl+1",
        "Ctrl+O",
        "Ctrl+W",
        "Alt+Space",
        "Meta+Q",
        "F12",
    ] {
        assert_eq!(
            validate_input(&Command::Press {
                tab_id: "tab".into(),
                observation_id: "obs".into(),
                keys: keys.into()
            }),
            Err(Error::InvalidInput),
            "{keys}"
        );
    }
    for keys in ["Ctrl+A", "Ctrl+Shift+Z", "Shift+Tab", "Enter", "ArrowDown"] {
        assert!(
            validate_input(&Command::Press {
                tab_id: "tab".into(),
                observation_id: "obs".into(),
                keys: keys.into()
            })
            .is_ok()
        );
    }
    for address in [
        "chrome://settings",
        "file:///private",
        "devtools://devtools",
        "https://user:password@fixture.test",
    ] {
        assert_eq!(
            require_page_url(&json!({"frameTree":{"frame":{"url":address}}})),
            Err(Error::TabDenied)
        );
    }
}

#[tokio::test]
async fn failed_input_release_retains_exact_mouse_and_key_obligations_for_retry() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::{accept_async, tungstenite::Message};
    for mouse in [true, false] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let worker = Server(tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(socket).await.unwrap();
            let mut first = true;
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let request: Value = serde_json::from_str(&text).unwrap();
                assert!(matches!(
                    request["params"]["type"].as_str(),
                    Some("mouseReleased" | "keyUp")
                ));
                let mut response = json!({"id":request["id"],"sessionId":request["sessionId"]});
                if first {
                    response["error"] = json!({"code":-32000,"message":"fixture release failure"});
                    first = false;
                } else {
                    response["result"] = json!({});
                }
                socket
                    .send(Message::Text(response.to_string().into()))
                    .await
                    .unwrap();
            }
        }));
        let conn = Connection::connect(&format!("ws://{address}"))
            .await
            .unwrap();
        conn.registry().register_session("fixture-session", "page");
        let mut state = TabAutomation {
            session: "fixture-session".into(),
            pressed: mouse.then_some(input::Point { x: 1.0, y: 1.0 }),
            keys: vec![
                (
                    input::KeyChord {
                        modifiers: 2,
                        key: "Control".into(),
                        code: "ControlLeft".into(),
                        vk: 17,
                    },
                    2,
                ),
                (input::parse_key_combo("Ctrl+A").unwrap(), 0),
            ],
            ..Default::default()
        };
        assert_eq!(
            release_inputs(&conn, &mut state).await,
            Err(Error::ExecutionFailed)
        );
        assert_eq!(state.pressed.is_some(), mouse);
        assert_eq!(state.keys.len(), 2);
        release_inputs(&conn, &mut state).await.unwrap();
        assert!(state.pressed.is_none() && state.keys.is_empty());
        conn.shutdown().await;
        drop(worker);
    }
}
