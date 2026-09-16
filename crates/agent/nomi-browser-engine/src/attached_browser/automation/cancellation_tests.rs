//! Stop at exact protocol response boundaries; no browser process is opened.
use super::*;
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cut {
    FinalLocate,
    FinalFocus,
    Move,
    Pressed,
}

async fn stop_at(cut: Cut) -> (Vec<String>, bool) {
    let cancel = CancellationToken::new();
    let stopped = cancel.clone();
    let effects = Arc::new(Mutex::new(Vec::<String>::new()));
    let captured = effects.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut worker = Server(tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(socket).await.unwrap();
        let mut locations = 0;
        let mut focuses = 0;
        let target = json!({"targetId":"fixture-target","type":"page","title":"Fixture","url":"https://fixture.test/","attached":true,"canAccessOpener":false});
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let request: Value = serde_json::from_str(&text).unwrap();
            let result = match request["method"].as_str().unwrap() {
                "Target.getTargets" => json!({"targetInfos":[target]}),
                "Target.getTargetInfo" => json!({"targetInfo":target}),
                "Page.getFrameTree" => {
                    json!({"frameTree":{"frame":{"id":"fixture-frame","loaderId":"fixture-loader","url":"https://fixture.test/"}}})
                }
                "Page.bringToFront" | "Page.enable" => json!({}),
                "Emulation.setFocusEmulationEnabled" => {
                    assert_eq!(request["sessionId"], "fixture-session");
                    assert_eq!(request["params"]["enabled"], true);
                    json!({})
                }
                "Runtime.callFunctionOn" => {
                    let script = request["params"]["functionDeclaration"].as_str().unwrap();
                    let value = if script.contains(native_semantic::LOCATE) {
                        locations += 1;
                        if cut == Cut::FinalLocate && locations == 2 {
                            stopped.cancel();
                        }
                        json!({"x":10.0,"y":20.0})
                    } else if script.contains(native_semantic::IS_FOCUSED) {
                        focuses += 1;
                        if cut == Cut::FinalFocus && focuses == 2 {
                            stopped.cancel();
                        }
                        json!(true)
                    } else if script.contains("width:innerWidth") {
                        json!({"width":800,"height":600})
                    } else if script.contains(native_semantic::HIGHLIGHT) {
                        json!(true)
                    } else {
                        panic!("unexpected read-side script: {script}");
                    };
                    json!({"result":{"value":value}})
                }
                "Input.dispatchMouseEvent" => {
                    let kind = request["params"]["type"].as_str().unwrap();
                    captured.lock().unwrap().push(kind.into());
                    if (cut == Cut::Move && kind == "mouseMoved")
                        || (cut == Cut::Pressed && kind == "mousePressed")
                    {
                        stopped.cancel();
                    }
                    json!({})
                }
                "Input.insertText" => {
                    captured.lock().unwrap().push("insertText".into());
                    json!({})
                }
                other => panic!("unexpected protocol method: {other}"),
            };
            // Cancellation is requested while the real transport caller is
            // awaiting this response, not before execute_granted is entered.
            let mut response = json!({"id":request["id"],"result":result});
            if let Some(session) = request.get("sessionId") {
                response["sessionId"] = session.clone();
            }
            socket
                .send(Message::Text(response.to_string().into()))
                .await
                .unwrap();
        }
    }));
    let connection = Connection::connect(&format!("ws://{address}"))
        .await
        .unwrap();
    connection
        .registry()
        .register_session("fixture-session", "page");
    let browser = AttachedBrowser {
        state: Arc::new(Mutex::new(crate::attached_browser::AttachedState {
            connection: Some(connection.clone()),
            retirement: None,
            offered_tabs: Default::default(),
            retirement_pause: None,
            pending: Default::default(),
            automation: [(
                "fixture-target".into(),
                TabAutomation {
                    session: "fixture-session".into(),
                    frame: "fixture-frame".into(),
                    loader: "fixture-loader".into(),
                    object: "fixture-object".into(),
                    observation: "fixture-observation".into(),
                    refs: ["fixture-ref".into()].into(),
                    ..Default::default()
                },
            )]
            .into(),
        })),
        operations: Arc::new(tokio::sync::Mutex::new(())),
        incarnation: "fixture-incarnation".into(),
        browser_identity: [0; 32],
        chromium_major: 144,
    };
    let choices = browser.tabs_for_user().await.unwrap();
    let grant = browser.grant_tab(&choices.tabs[0].choice_id).await.unwrap();
    let command = match cut {
        Cut::FinalFocus => Command::Type {
            tab_id: grant.id().into(),
            observation_id: "fixture-observation".into(),
            ref_id: "fixture-ref".into(),
            text: "must not be inserted".into(),
            replace: false,
        },
        Cut::Move => Command::Scroll {
            tab_id: grant.id().into(),
            observation_id: "fixture-observation".into(),
            delta_x: 0.0,
            delta_y: 200.0,
        },
        _ => Command::Click {
            tab_id: grant.id().into(),
            observation_id: "fixture-observation".into(),
            ref_id: "fixture-ref".into(),
        },
    };
    assert_eq!(
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            browser.execute_granted(&grant, command, &cancel)
        )
        .await
        .unwrap(),
        Err(Error::Cancelled)
    );
    assert!(
        cancel.is_cancelled(),
        "the protocol cutpoint was actually reached"
    );
    let pressed = browser.state.lock().unwrap().automation["fixture-target"]
        .pressed
        .is_some();
    browser.disconnect().await.unwrap();
    connection.shutdown().await;
    (&mut worker.0).await.unwrap();
    let effects = effects.lock().unwrap().clone();
    (effects, pressed)
}

#[tokio::test]
async fn stop_during_final_locate_does_not_start_mouse_press() {
    let (effects, pressed) = stop_at(Cut::FinalLocate).await;
    assert_eq!(effects, ["mouseMoved"]);
    assert!(!pressed);
}
#[tokio::test]
async fn stop_during_final_focus_check_does_not_insert_text() {
    let (effects, pressed) = stop_at(Cut::FinalFocus).await;
    assert_eq!(effects, ["mouseMoved", "mousePressed", "mouseReleased"]);
    assert!(!pressed);
}
#[tokio::test]
async fn stop_during_mouse_move_does_not_dispatch_wheel() {
    let (effects, pressed) = stop_at(Cut::Move).await;
    assert_eq!(effects, ["mouseMoved"]);
    assert!(!pressed);
}
#[tokio::test]
async fn stop_after_mouse_press_still_completes_exact_release() {
    let (effects, pressed) = stop_at(Cut::Pressed).await;
    assert_eq!(effects, ["mouseMoved", "mousePressed", "mouseReleased"]);
    assert!(!pressed);
}
