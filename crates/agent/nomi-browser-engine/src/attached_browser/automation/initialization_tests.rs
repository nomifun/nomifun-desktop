//! A real local protocol transport, without starting or touching a browser.
//! Failed Evaluate replies must not discard the remote object-group obligation.
use super::*;
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[derive(Clone, Copy)]
enum EvaluateFailure {
    ProtocolError,
    MissingObjectId,
    JavascriptException,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    CreateWorld,
    Evaluate(String),
    Release(String),
    Detach(String),
}

struct FixtureServer(tokio::task::JoinHandle<()>);
impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn initialization_failure_retains_group(failure: EvaluateFailure) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut worker = FixtureServer(tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(socket).await.unwrap();
        let target = json!({
            "targetId":"fixture-target", "type":"page", "title":"Fixture",
            "url":"https://fixture.test/", "attached":true, "canAccessOpener":false
        });
        while let Some(Ok(Message::Text(text))) = socket.next().await {
            let request: Value = serde_json::from_str(&text).unwrap();
            let method = request["method"].as_str().unwrap();
            let params = &request["params"];
            let mut protocol_error = false;
            let result = match method {
                "Target.getTargets" => json!({"targetInfos":[target]}),
                "Target.getTargetInfo" => json!({"targetInfo":target}),
                "Target.attachToTarget" => {
                    assert_eq!(params["targetId"], "fixture-target");
                    json!({"sessionId":"fixture-session"})
                }
                "Page.getFrameTree" => json!({"frameTree":{"frame":{
                    "id":"fixture-frame", "loaderId":"fixture-loader", "url":"https://fixture.test/"
                }}}),
                "Page.bringToFront" | "Page.enable" => json!({}),
                "Page.createIsolatedWorld" => {
                    assert_eq!(params["frameId"], "fixture-frame");
                    captured.lock().unwrap().push(Event::CreateWorld);
                    json!({"executionContextId":7})
                }
                "Runtime.evaluate" => {
                    let group = params["objectGroup"].as_str().unwrap();
                    assert!(!group.is_empty());
                    assert_eq!(params["contextId"], 7);
                    captured.lock().unwrap().push(Event::Evaluate(group.into()));
                    match failure {
                        EvaluateFailure::ProtocolError => {
                            protocol_error = true;
                            json!({})
                        }
                        EvaluateFailure::MissingObjectId => json!({"result":{"type":"object"}}),
                        EvaluateFailure::JavascriptException => json!({
                            "result":{"type":"undefined"},
                            "exceptionDetails":{"text":"fixture initialization failed"}
                        }),
                    }
                }
                "Runtime.releaseObjectGroup" => {
                    assert_eq!(request["sessionId"], "fixture-session");
                    let group = params["objectGroup"].as_str().unwrap();
                    captured.lock().unwrap().push(Event::Release(group.into()));
                    json!({})
                }
                "Target.detachFromTarget" => {
                    let session = params["sessionId"].as_str().unwrap();
                    captured.lock().unwrap().push(Event::Detach(session.into()));
                    json!({})
                }
                other => panic!("unexpected protocol method after failed initialization: {other}"),
            };
            let mut response = if protocol_error {
                json!({"id":request["id"],"error":{"code":-32000,"message":"fixture Evaluate failed"}})
            } else {
                json!({"id":request["id"],"result":result})
            };
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
    let browser = AttachedBrowser {
        state: Arc::new(Mutex::new(crate::attached_browser::AttachedState {
            connection: Some(connection.clone()),
            retirement: None,
            automation: Default::default(),
            pending: Default::default(),
            retirement_pause: None,
        })),
        operations: Arc::new(tokio::sync::Mutex::new(())),
        incarnation: "fixture-incarnation".into(),
        browser_identity: [0; 32],
        chromium_major: 144,
    };
    let grant = browser
        .tabs_for_provider()
        .await
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .grant;
    let cancel = CancellationToken::new();
    let mut groups = Vec::new();
    for _ in 0..2 {
        assert_eq!(
            tokio::time::timeout(
                std::time::Duration::from_secs(3),
                browser.execute_granted(
                    &grant,
                    Command::Observe {
                        tab_id: grant.id().into()
                    },
                    &cancel,
                )
            )
            .await
            .unwrap(),
            Err(Error::ExecutionFailed)
        );
        let state = browser.state.lock().unwrap();
        let automation = &state.automation["fixture-target"];
        assert!(
            automation.object.is_empty(),
            "no remote object handle was received"
        );
        assert!(
            !automation.group.is_empty(),
            "the exact group must remain owned despite a missing handle"
        );
        assert!(automation.observation.is_empty());
        assert!(automation.refs.is_empty());
        groups.push(automation.group.clone());
    }
    assert_ne!(groups[0], groups[1]);
    assert_eq!(
        *events.lock().unwrap(),
        [
            Event::CreateWorld,
            Event::Evaluate(groups[0].clone()),
            Event::Release(groups[0].clone()),
            Event::CreateWorld,
            Event::Evaluate(groups[1].clone()),
        ],
        "retry must release the precise previous group before creating another world/group"
    );
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        browser.release_granted(&grant),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        !browser
            .state
            .lock()
            .unwrap()
            .automation
            .contains_key("fixture-target")
    );
    assert_eq!(
        *events.lock().unwrap(),
        [
            Event::CreateWorld,
            Event::Evaluate(groups[0].clone()),
            Event::Release(groups[0].clone()),
            Event::CreateWorld,
            Event::Evaluate(groups[1].clone()),
            Event::Release(groups[1].clone()),
            Event::Detach("fixture-session".into()),
        ],
        "settlement must release even the handle-less final group before detaching the page session"
    );
    browser.disconnect().await.unwrap();
    connection.shutdown().await;
    tokio::time::timeout(std::time::Duration::from_secs(3), &mut worker.0)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn root_evaluate_protocol_error_keeps_exact_group_until_retry_and_settlement() {
    initialization_failure_retains_group(EvaluateFailure::ProtocolError).await;
}

#[tokio::test]
async fn root_evaluate_missing_object_id_keeps_exact_group_until_retry_and_settlement() {
    initialization_failure_retains_group(EvaluateFailure::MissingObjectId).await;
}

#[tokio::test]
async fn root_evaluate_javascript_exception_keeps_exact_group_until_retry_and_settlement() {
    initialization_failure_retains_group(EvaluateFailure::JavascriptException).await;
}
