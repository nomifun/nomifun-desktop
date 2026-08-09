//! End-to-end: a fake device walks the whole firmware flow against the real
//! gateway, with mocked speech and dispatch. Nothing here touches hardware or a
//! model provider, so it runs in ordinary local test loops.

use futures_util::{SinkExt, StreamExt};
use nomifun_robot::endpoint::LanEndpointSnapshot;
use nomifun_robot::registry::RobotRegistry;
use nomifun_robot::services::TurnEvent;
use nomifun_robot::services::mock::{MockDispatcher, MockSpeech};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

struct Harness {
    base: String,
    http: reqwest::Client,
    speech: Arc<MockSpeech>,
    dispatcher: Arc<MockDispatcher>,
    /// The advertiser reads this; dropping it would freeze the endpoint at its
    /// last value, so the harness owns it for the life of the test.
    _endpoint_tx: tokio::sync::watch::Sender<LanEndpointSnapshot>,
    _dir: tempfile::TempDir,
}

struct NullSink;
impl nomifun_realtime::UserEventSink for NullSink {
    fn send_to_user(
        &self,
        _user_id: &str,
        _event: nomifun_api_types::WebSocketMessage<serde_json::Value>,
    ) {
    }
}

/// Boot the gateway on a real loopback port: the device face, the management
/// face, the accept loop and the session actors, with only the model layer
/// mocked.
async fn boot() -> Harness {
    let dir = tempfile::tempdir().unwrap();
    let registry = Arc::new(RobotRegistry::load(dir.path()).await.unwrap());
    let speech = Arc::new(MockSpeech::new());
    let dispatcher = Arc::new(MockDispatcher::new());
    dispatcher.set_has_fallback(false);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // The advertiser must hand out this very port, or the device would be told
    // to connect somewhere nothing is listening.
    let (endpoint_tx, endpoint_rx) = tokio::sync::watch::channel(LanEndpointSnapshot {
        enabled: true,
        port,
        ipv4s: vec![std::net::Ipv4Addr::LOCALHOST],
    });
    let advertiser: Arc<dyn nomifun_robot::endpoint::EndpointAdvertiser> =
        Arc::new(nomifun_robot::endpoint::LanAdvertiser::new(endpoint_rx));

    let status = Arc::new(nomifun_robot::status::RobotStatusRegistry::new(
        nomifun_robot::events::RobotEventEmitter::new(Arc::new(NullSink)),
        "owner-1".to_owned(),
    ));
    let tools = Arc::new(nomifun_robot::tool_registry::RobotToolRegistry::default());
    let (source, acceptor) = nomifun_robot::lan_source::LanWsSource::new();

    let device_state = nomifun_robot::routes::RobotDeviceState {
        registry: registry.clone(),
        advertiser: advertiser.clone(),
        acceptor,
        speech: speech.clone(),
    };
    let admin_state = nomifun_robot::routes::RobotAdminState {
        registry: registry.clone(),
        status: status.clone(),
        advertiser,
    };

    let gateway = Arc::new(nomifun_robot::RobotGateway::new(
        nomifun_robot::session::SessionDeps {
            registry,
            status,
            speech: speech.clone(),
            dispatcher: dispatcher.clone(),
            tools,
        },
    ));
    tokio::spawn(gateway.serve(vec![source]));

    let app = axum::Router::new()
        .nest("/robot", nomifun_robot::routes::device_router(device_state))
        .merge(nomifun_robot::routes::admin_router(admin_state));
    tokio::spawn(async move {
        // ConnectInfo is required: the OTA handler picks an interface by peer IP.
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    Harness {
        base: format!("http://127.0.0.1:{port}"),
        http: reqwest::Client::new(),
        speech,
        dispatcher,
        _endpoint_tx: endpoint_tx,
        _dir: dir,
    }
}

type Socket = WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Build a WebSocket upgrade request with the device's headers.
///
/// `IntoClientRequest` for a pre-built `http::Request` passes it through
/// verbatim, so the handshake headers are ours to supply — which is also what the
/// firmware does.
fn ws_request(
    url: &str,
    headers: &[(&str, &str)],
) -> tokio_tungstenite::tungstenite::http::Request<()> {
    use tokio_tungstenite::tungstenite::handshake::client::generate_key;
    use tokio_tungstenite::tungstenite::http::Uri;

    let uri: Uri = url.parse().expect("test urls are well formed");
    let authority = uri.authority().expect("test urls carry an authority").as_str();
    let mut builder = tokio_tungstenite::tungstenite::http::Request::builder()
        .uri(url)
        .header("Host", authority)
        .header("Connection", "Upgrade")
        .header("Upgrade", "websocket")
        .header("Sec-WebSocket-Version", "13")
        .header("Sec-WebSocket-Key", generate_key());
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder.body(()).unwrap()
}

async fn send_text(socket: &mut Socket, raw: &str) {
    socket.send(Message::Text(raw.into())).await.unwrap();
}

async fn next_frame(socket: &mut Socket) -> Message {
    loop {
        match socket.next().await {
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(message)) => return message,
            _ => return Message::Close(None),
        }
    }
}

async fn next_json(socket: &mut Socket) -> Value {
    loop {
        if let Message::Text(raw) = next_frame(socket).await {
            return serde_json::from_str(&raw).unwrap();
        }
    }
}

/// Encode `ms` of 16 kHz audio into 60 ms uplink packets, as the device would.
///
/// `loud` audio has to read as speech to the **real** Silero endpointer the
/// session builds, so it is a glottal pulse train under two sweeping formants; a
/// static tone mix scores ~1% and would never end a turn. Duplicated from the
/// uplink unit tests on purpose: an integration test is a separate crate and
/// cannot reach into `#[cfg(test)]` modules.
fn uplink_packets(ms: u32, loud: bool) -> Vec<Vec<u8>> {
    let n = (16_000u64 * ms as u64 / 1000) as usize;
    let tau = std::f32::consts::TAU;
    let pcm: Vec<i16> = (0..n)
        .map(|i| {
            if !loud {
                return 0;
            }
            let t = i as f32 / 16_000.0;
            let f0 = 120.0 + 30.0 * (t * 4.0 * tau).sin();
            let f1 = 500.0 + 300.0 * (t * 3.0 * tau).sin();
            let f2 = 1500.0 + 500.0 * (t * 2.0 * tau).cos();
            let mut v = 0.0;
            for harmonic in 1..=20 {
                let f = f0 * harmonic as f32;
                let a = (-((f - f1) / 250.0).powi(2)).exp()
                    + 0.6 * (-((f - f2) / 350.0).powi(2)).exp();
                v += a * (t * f * tau).sin();
            }
            let enveloped = v * 0.5 * (0.6 + 0.4 * (t * 3.5 * tau).sin());
            (enveloped.clamp(-1.0, 1.0) * 9000.0) as i16
        })
        .collect();
    nomifun_robot::audio::OpusStreamEncoder::new_uplink_for_test()
        .unwrap()
        .encode_frames(&pcm)
        .unwrap()
}

/// One full device lifetime: report, activate, claim, talk, get interrupted, go
/// away.
#[tokio::test]
async fn a_device_reports_gets_claimed_talks_and_is_interrupted() {
    let h = boot().await;

    // 1. OTA report: fresh device, so an activation code and a token come back.
    let ota: Value = h
        .http
        .post(format!("{}/robot/ota", h.base))
        .header("Device-Id", "aa:bb:cc:dd:ee:ff")
        .header("Client-Id", "3f2b9c1e-0000-4000-8000-000000000001")
        .json(&serde_json::json!({
            "version": 2,
            "application": { "version": "1.9.0" },
            "board": { "type": "esp32-s3n16r8-emoji" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        ota.get("mqtt").is_none(),
        "an mqtt object would make the firmware pick MQTT with no fallback"
    );
    let token = ota["websocket"]["token"].as_str().unwrap().to_owned();
    let ws_url = ota["websocket"]["url"].as_str().unwrap().to_owned();
    let code = ota["activation"]["code"].as_str().unwrap().to_owned();
    assert!(
        ws_url.starts_with("ws://127.0.0.1:"),
        "the advertised endpoint must be the port we are serving: {ws_url}"
    );

    // 2. Activation polling says 202 until a human claims it.
    let pending = h
        .http
        .post(format!("{}/robot/ota/activate", h.base))
        .header("Device-Id", "aa:bb:cc:dd:ee:ff")
        .send()
        .await
        .unwrap();
    assert_eq!(pending.status(), 202);

    // 3. The UI claims the code for a companion.
    let claimed = h
        .http
        .post(format!("{}/api/robots/claim", h.base))
        .json(&serde_json::json!({
            "code": code,
            "companion_id": "0190f5fe-7c00-7a00-8000-0000000000aa"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(claimed.status(), 200);

    let done = h
        .http
        .post(format!("{}/robot/ota/activate", h.base))
        .header("Device-Id", "aa:bb:cc:dd:ee:ff")
        .send()
        .await
        .unwrap();
    assert_eq!(done.status(), 200);

    // 4. Connect the audio channel and handshake.
    h.speech.push_transcript("讲个故事");
    h.dispatcher.script_turn(vec![
        TurnEvent::Text("[winking] 从前有座山。".into()),
        TurnEvent::Text("山上有座庙。".into()),
        TurnEvent::Done,
    ]);

    let request = ws_request(
        &ws_url,
        &[
            ("Authorization", &format!("Bearer {token}")),
            ("Device-Id", "aa:bb:cc:dd:ee:ff"),
            ("Client-Id", "3f2b9c1e-0000-4000-8000-000000000001"),
            ("Protocol-Version", "1"),
        ],
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();

    send_text(
        &mut socket,
        r#"{"type":"hello","version":1,"transport":"websocket","features":{"mcp":true},"audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}"#,
    )
    .await;
    let hello = next_json(&mut socket).await;
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["audio_params"]["sample_rate"], 24000);
    assert_eq!(hello["audio_params"]["frame_duration"], 60);

    // 5. Speak: listen start, 300 ms of audio, then silence to end the turn.
    send_text(
        &mut socket,
        r#"{"session_id":"s","type":"listen","state":"start","mode":"auto"}"#,
    )
    .await;
    for packet in uplink_packets(300, true) {
        socket.send(Message::Binary(packet.into())).await.unwrap();
    }
    for packet in uplink_packets(900, false) {
        socket.send(Message::Binary(packet.into())).await.unwrap();
    }

    // 6. Expect the documented downlink sequence, and audio as bare binary.
    let mut seen_stt = false;
    let mut seen_sentences = 0;
    let mut audio_frames = 0;
    // Both sentence headers land before the pacer has released a single 60 ms
    // frame, so audio is waited for explicitly rather than assumed.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    while tokio::time::Instant::now() < deadline && (seen_sentences < 2 || audio_frames == 0) {
        match next_frame(&mut socket).await {
            Message::Text(raw) => {
                let value: Value = serde_json::from_str(&raw).unwrap();
                match value["type"].as_str() {
                    Some("stt") => {
                        assert_eq!(value["text"], "讲个故事");
                        seen_stt = true;
                    }
                    Some("llm") => panic!(
                        "a healthy turn sets no face; model text is not an expression channel: {value}"
                    ),
                    Some("tts") if value["state"] == "sentence_start" => {
                        let text = value["text"].as_str().unwrap();
                        assert!(
                            !text.contains('[') && !text.contains('】'),
                            "a bracketed annotation must never reach the screen: {text}"
                        );
                        seen_sentences += 1;
                    }
                    _ => {}
                }
            }
            Message::Binary(_) => audio_frames += 1,
            Message::Close(_) => panic!("the gateway hung up mid-turn"),
            _ => {}
        }
    }
    assert!(seen_stt, "the transcript must reach the device");
    assert_eq!(seen_sentences, 2);
    assert!(
        audio_frames > 0,
        "synthesised audio must arrive as binary frames"
    );

    // 7. Interrupt: after the stop that acknowledges the abort, not one more
    //    audio frame may arrive — a trailing word is what a user hears as a
    //    robot that ignored them.
    send_text(
        &mut socket,
        r#"{"session_id":"s","type":"abort","reason":"wake_word_detected"}"#,
    )
    .await;
    let mut saw_stop = false;
    let quiet_after = tokio::time::Instant::now() + std::time::Duration::from_millis(600);
    while tokio::time::Instant::now() < quiet_after {
        match tokio::time::timeout(
            std::time::Duration::from_millis(120),
            next_frame(&mut socket),
        )
        .await
        {
            Ok(Message::Text(raw)) => {
                let value: Value = serde_json::from_str(&raw).unwrap();
                if value["type"] == "tts" && value["state"] == "stop" {
                    saw_stop = true;
                }
            }
            Ok(Message::Binary(_)) => {
                assert!(
                    !saw_stop,
                    "no audio may follow the tts stop that acknowledges an abort"
                );
            }
            _ => {}
        }
    }
    assert!(saw_stop, "abort must be acknowledged with tts stop");

    // 8. Status is visible to the UI, and going away flips it to offline.
    let statuses: Value = h
        .http
        .get(format!("{}/api/robots/statuses", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(statuses["statuses"][0]["robot_id"], "aa:bb:cc:dd:ee:ff");

    drop(socket);
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let statuses: Value = h
        .http
        .get(format!("{}/api/robots/statuses", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(statuses["statuses"][0]["phase"], "offline");
}

#[tokio::test]
async fn an_unclaimed_device_is_refused_a_session() {
    let h = boot().await;
    let ota: Value = h
        .http
        .post(format!("{}/robot/ota", h.base))
        .header("Device-Id", "aa:bb:cc:dd:ee:02")
        .header("Client-Id", "cid")
        .json(&serde_json::json!({
            "version": 2,
            "application": { "version": "1.9.0" },
            "board": { "type": "b" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token = ota["websocket"]["token"].as_str().unwrap().to_owned();
    let ws_url = ota["websocket"]["url"].as_str().unwrap().to_owned();

    let request = ws_request(&ws_url, &[("Authorization", &format!("Bearer {token}"))]);
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    send_text(
        &mut socket,
        r#"{"type":"hello","version":1,"transport":"websocket"}"#,
    )
    .await;
    // The gateway hangs up instead of issuing a session.
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        next_frame(&mut socket),
    )
    .await;
    assert!(
        match outcome {
            Ok(Message::Text(raw)) => !raw.contains("\"hello\""),
            _ => true,
        },
        "an unbound robot must never receive a server hello"
    );
}

#[tokio::test]
async fn a_bad_token_cannot_open_the_websocket() {
    let h = boot().await;
    let request = ws_request(
        &format!("{}/robot/v1", h.base.replace("http://", "ws://")),
        &[("Authorization", "Bearer not-a-token")],
    );
    let error = tokio_tungstenite::connect_async(request)
        .await
        .expect_err("the upgrade must be rejected before any frame is exchanged");
    assert!(
        matches!(
            error,
            tokio_tungstenite::tungstenite::Error::Http(ref response)
                if response.status() == 401
        ),
        "expected a 401, got {error:?}"
    );
}
