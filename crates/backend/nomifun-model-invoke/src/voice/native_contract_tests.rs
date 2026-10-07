//! This verifies the production HTTP/sideband wire, not WebRTC device behavior.
use super::contract_tests::{
    ack_live_initialization, connection, next_event, next_json, request, send,
};
use nomifun_voice_contracts::voice::*;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

async fn native_create_request(stream: &mut tokio::net::TcpStream) -> Value {
    native_create_request_with_headers(stream).await.0
}

async fn native_create_request_with_headers(stream: &mut tokio::net::TcpStream) -> (Value, String) {
    let mut bytes = Vec::new();
    let (header_end, length) = loop {
        let mut buffer = [0u8; 4096];
        let n = stream.read(&mut buffer).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
        assert!(bytes.len() < 128 * 1024);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            assert!(headers.starts_with("POST /v1/live/sessions HTTP/1.1"));
            let length = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    if key.eq_ignore_ascii_case("content-length") {
                        Some(value.trim().parse::<usize>().unwrap())
                    } else {
                        None
                    }
                })
                .unwrap();
            break (end + 4, length);
        }
    };
    while bytes.len() < header_end + length {
        let mut buffer = [0u8; 4096];
        let n = stream.read(&mut buffer).await.unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&buffer[..n]);
    }
    (
        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap(),
        String::from_utf8(bytes[..header_end].to_vec()).unwrap(),
    )
}

#[tokio::test]
async fn native_rotated_create_pins_successful_project_credential_for_sideband() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        for (credential, status, payload) in [
            (
                "test-only-throttled-project",
                "429 Too Many Requests",
                json!({"error":{"message":"throttled"}}),
            ),
            (
                "test-only-selected-project",
                "200 OK",
                json!({"session":{"id":"selected-project-session","model":"gpt-live-1","delegation":{"type":"client"}},"transport":{"type":"webrtc","sdp":"answer"}}),
            ),
        ] {
            let (mut http, _) = listener.accept().await.unwrap();
            let (_, headers) = native_create_request_with_headers(&mut http).await;
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains(&format!("authorization: bearer {credential}"))
            );
            let payload = payload.to_string();
            http.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len()).as_bytes()).await.unwrap();
            http.shutdown().await.unwrap();
        }
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            |req: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(
                    req.uri().path(),
                    "/v1/live/sessions/selected-project-session/attach"
                );
                assert_eq!(
                    req.headers().get("authorization").unwrap(),
                    "Bearer test-only-selected-project"
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        ack_live_initialization(&mut socket).await;
        assert_eq!(next_json(&mut socket).await["type"], "session.close");
        send(
            &mut socket,
            json!({"type":"session.closed","reason":"close_requested"}),
        )
        .await;
        let _ = socket.close(None).await;
    });
    let mut credentials = connection(port);
    credentials.auth.credentials =
        json!({"api_keys":["test-only-throttled-project","test-only-selected-project"]});
    let model =
        super::create_openai_live_model_port(credentials, json!({}), "gpt-live-1".into()).unwrap();
    let mut open = request("gpt-live-1", VoiceTransportPreference::NativeWebrtc);
    open.native_offer = Some("offer".into());
    let session = model
        .open(
            open,
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(3),
        )
        .await
        .unwrap();
    assert!(
        session
            .shutdown(VoiceCloseReason::UserEnded)
            .await
            .finalization_confirmed
    );
    server.await.unwrap();
}

#[tokio::test]
async fn live_native_create_sideband_and_attachment_rebuild_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (mut http, _) = listener.accept().await.unwrap();
        let body = native_create_request(&mut http).await;
        assert_eq!(body["transport"]["type"], "webrtc");
        assert_eq!(body["transport"]["sdp"], "test-offer-sdp");
        assert!(body["session"]["audio"].get("format").is_none());
        assert_eq!(
            body["session"]["client"]["data_channel"]["allowed_client_events"],
            json!([])
        );
        let response=json!({"session":{"id":"opaque-native-id","model":"gpt-live-1","delegation":{"type":"client"}},"transport":{"type":"webrtc","sdp":"test-answer-sdp"}}).to_string();
        http.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
        http.shutdown().await.unwrap();
        drop(http);
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            |req: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(
                    req.uri().path(),
                    "/v1/live/sessions/opaque-native-id/attach"
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        ack_live_initialization(&mut socket).await;
        send(
            &mut socket,
            json!({"type":"session.output_audio.delta","delta":"AAA=","start_ms":0,"end_ms":1}),
        )
        .await;
        send(&mut socket,json!({"type":"session.input_transcript.delta","event_id":"native-caption","delta":"hello","start_ms":0,"end_ms":100})).await;
        // The first event must be this explicit control, never session.start
        // or duplicate input_audio append on the already running sideband.
        assert_eq!(
            next_json(&mut socket).await["type"],
            "session.input_audio.mute"
        );
        assert_eq!(
            next_json(&mut socket).await["type"],
            "session.instructions.append"
        );
        send(
            &mut socket,
            json!({"type":"session.instructions.appended","client_event_id":"unknown"}),
        )
        .await;
        send(
            &mut socket,
            json!({"type":"session.output_audio.delta","delta":"AAA=","start_ms":100,"end_ms":101}),
        )
        .await;
        assert_eq!(next_json(&mut socket).await["type"], "session.close");
        send(
            &mut socket,
            json!({"type":"session.closed","reason":"close_requested","usage":{"seconds":1}}),
        )
        .await;
        let _ = socket.close(None).await;
    });
    let model =
        super::create_openai_live_model_port(connection(port), json!({}), "gpt-live-1".into())
            .unwrap();
    let mut open = request("gpt-live-1", VoiceTransportPreference::NativeWebrtc);
    open.native_offer = Some("test-offer-sdp".into());
    let mut session = model
        .open(
            open,
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(3),
        )
        .await
        .unwrap();
    assert!(session.negotiation.input_spec.is_none());
    assert!(session.negotiation.output_spec.is_none());
    assert_eq!(
        session
            .negotiation
            .native_attachment
            .as_ref()
            .unwrap()
            .answer_sdp,
        "test-answer-sdp"
    );
    loop {
        match next_event(&mut session).await {
            VoiceModelEvent::Transcript { .. } => break,
            VoiceModelEvent::Audio { .. } => {
                panic!("native reflected audio must never be forwarded")
            }
            _ => {}
        }
    }
    session
        .input
        .try_control(VoiceControl::MuteInput { muted: true })
        .unwrap();
    session
        .input
        .try_control(VoiceControl::InterruptOutput {
            output_generation: 2,
            played: None,
        })
        .unwrap();
    loop {
        match next_event(&mut session).await {
            VoiceModelEvent::NativeAttachmentRequired {
                output_generation: 2,
            } => break,
            VoiceModelEvent::OutputBoundary { .. } => {
                panic!("instruction acknowledgment cannot unlock native sink")
            }
            _ => {}
        }
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(100), session.events.recv())
            .await
            .is_err()
    );
    assert!(
        session
            .shutdown(VoiceCloseReason::UserEnded)
            .await
            .finalization_confirmed
    );
    server.await.unwrap();
}
