//! Local wire fixtures exercise the production ports, not live model quality.
use crate::{AuthMaterial, AuthScheme, ResolvedConnection};
use futures_util::{SinkExt, StreamExt};
use nomifun_voice_contracts::voice::*;
use nomifun_voice_core::{VoiceModelSession, VoiceOpenRequest};
use serde_json::{Value, json};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::Instant;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

pub(super) fn connection(port: u16) -> ResolvedConnection {
    ResolvedConnection {
        role: "default".into(),
        base_url: format!("http://127.0.0.1:{port}/v1"),
        auth: AuthMaterial {
            scheme: AuthScheme::Bearer,
            credentials: json!({"api_keys":["test-only-private-credential"]}),
        },
        extra: json!({}),
    }
}
pub(super) fn request(model: &str, transport: VoiceTransportPreference) -> VoiceOpenRequest {
    VoiceOpenRequest{voice_session_id:"voice-test".into(),activation_epoch:1,output_generation:1,
        route_identity:serde_json::from_value(json!({"route_id":"r","revision":1,"record_digest":"0".repeat(64),"adapter_contract_ref":"test@1"})).unwrap(),
        model:model.into(),instructions:"Use the application work bridge.".into(),
        tools:vec![VoiceToolDefinition{name:"start".into(),description:"Start work".into(),parameters:json!({"type":"object","required":["text"],"properties":{"text":{"type":"string"}}})}],
        transport,native_offer:None,work_context:None,initial_facts:Vec::new(),
        replay_scope:VoiceReplayScope{agent_session_id:"test-agent".into(),binding_version:1,context_floor:0},initial_local_replay:None}
}
pub(super) async fn next_json<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>) -> Value
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    loop {
        match socket.next().await.unwrap().unwrap() {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(payload) => socket.send(Message::Pong(payload)).await.unwrap(),
            _ => {}
        }
    }
}
pub(super) async fn send<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>, value: Value)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    socket
        .send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}
pub(super) async fn next_event(session: &mut VoiceModelSession) -> VoiceModelEvent {
    tokio::time::timeout(Duration::from_secs(3), session.events.recv())
        .await
        .unwrap()
        .unwrap()
}

pub(super) async fn ack_live_initialization<S>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
) -> Vec<String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut contents = Vec::new();
    loop {
        let command = next_json(socket).await;
        assert_eq!(command["type"], "session.thinking.append");
        assert!(command.get("speech_source").is_none());
        let content = command["content"].as_str().unwrap().to_owned();
        let done = content.starts_with("Application context initialized.");
        send(socket,json!({"type":"session.thinking.appended","client_event_id":command["event_id"],"start_ms":0,"end_ms":0})).await;
        contents.push(content);
        if done {
            return contents;
        }
    }
}

#[tokio::test]
async fn stepfun_production_port_tool_fact_interrupt_and_close_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (fact_seen_tx, fact_seen_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_hdr_async(
            stream,
            |req: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(req.uri().path(), "/v1/realtime");
                assert!(
                    req.uri()
                        .query()
                        .unwrap()
                        .contains("model=stepaudio-3-realtime-preview")
                );
                assert_eq!(
                    req.headers()["authorization"],
                    "Bearer test-only-private-credential"
                );
                Ok(response)
            },
        )
        .await
        .unwrap();
        let update = next_json(&mut socket).await;
        assert_eq!(update["type"], "session.update");
        assert_eq!(update["session"]["turn_detection"]["type"], "server_vad");
        assert_eq!(update["session"]["tools"][0]["function"]["name"], "start");
        send(&mut socket,json!({"type":"session.updated","session":{"id":"step-session","model":"stepaudio-3-realtime-preview","input_audio_format":"pcm16","output_audio_format":"pcm16"}})).await;
        send(
            &mut socket,
            json!({"type":"response.created","response":{"id":"old"}}),
        )
        .await;
        send(&mut socket,json!({"type":"response.function_call_arguments.delta","call_id":"call-a","arguments":"{"})).await;
        send(&mut socket,json!({"type":"response.function_call_arguments.done","call_id":"call-a","name":"start","arguments":"{\"text\":\"Do work\"}"})).await;
        send(
            &mut socket,
            json!({"type":"error","error":{"message":"echo test-only-private-credential"}}),
        )
        .await;
        let fact = next_json(&mut socket).await;
        assert_eq!(fact["item"]["type"], "function_call_output");
        assert_eq!(fact["item"]["call_id"], "call-a");
        let _ = fact_seen_tx.send(());
        let interrupt = next_json(&mut socket).await;
        assert_eq!(interrupt["type"], "response.cancel");
        send(
            &mut socket,
            json!({"type":"response.audio.delta","response_id":"old","delta":"AAA="}),
        )
        .await;
        send(
            &mut socket,
            json!({"type":"response.cancelled","response_id":"old"}),
        )
        .await;
        send(&mut socket,json!({"type":"response.audio_transcript.delta","response_id":"old","item_id":"old-item","delta":"revoked caption"})).await;
        send(&mut socket,json!({"type":"response.function_call_arguments.done","response_id":"old","call_id":"revoked-call","name":"start","arguments":"{\"text\":\"late revoked work\"}"})).await;
        send(
            &mut socket,
            json!({"type":"response.created","response":{"id":"new"}}),
        )
        .await;
        send(
            &mut socket,
            json!({"type":"response.audio.delta","response_id":"new","delta":"AAA="}),
        )
        .await;
        while let Some(message) = socket.next().await {
            let message = message.unwrap();
            if let Message::Text(text) = &message {
                assert!(
                    !text.contains("late fact"),
                    "revoked fact must not start speech on the new generation"
                );
            }
            if matches!(message, Message::Close(_)) {
                let _ = socket.flush().await;
                break;
            }
        }
    });
    let model = super::create_stepfun_model_port(
        connection(port),
        json!({}),
        "stepaudio-3-realtime-preview".into(),
    )
    .unwrap();
    let mut session = model
        .open(
            request(
                "stepaudio-3-realtime-preview",
                VoiceTransportPreference::Relay,
            ),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(3),
        )
        .await
        .unwrap();
    let mut saw_tool = false;
    let mut saw_redacted = false;
    while !saw_tool || !saw_redacted {
        match next_event(&mut session).await {
            VoiceModelEvent::WorkTrigger {
                trigger:
                    WorkTrigger::TypedToolCall {
                        upstream_trigger_id,
                        arguments,
                        ..
                    },
            } => {
                assert_eq!(upstream_trigger_id, "call-a");
                assert_eq!(arguments["text"], "Do work");
                saw_tool = true;
            }
            VoiceModelEvent::ControlRejected { error } => {
                assert!(!error.message.contains("test-only-private-credential"));
                assert!(error.message.contains("[REDACTED]"));
                saw_redacted = true;
            }
            _ => {}
        }
    }
    session
        .input
        .try_control(VoiceControl::InjectFact {
            fact: VerifiedVoiceFact {
                output_generation: None,
                work_context: None,
                speech_source: None,
                correlation_id: "work".into(),
                upstream_trigger_id: Some("call-a".into()),
                canonical_receipt_id: "receipt".into(),
                content: "Accepted; work continues.".into(),
                speak: false,
            },
        })
        .unwrap();
    fact_seen_rx.await.unwrap();
    session
        .input
        .try_control(VoiceControl::InterruptOutput {
            output_generation: 2,
            played: None,
        })
        .unwrap();
    loop {
        match next_event(&mut session).await {
            VoiceModelEvent::Audio { segment_id, frame } => {
                assert_eq!(segment_id, "new");
                assert_eq!(frame.output_generation, 2);
                break;
            }
            VoiceModelEvent::Transcript { fragment } => {
                assert!(!fragment.text.contains("revoked caption"))
            }
            VoiceModelEvent::WorkTrigger { trigger } => {
                assert_ne!(trigger.upstream_trigger_id(), "revoked-call")
            }
            _ => {}
        }
    }
    session
        .input
        .try_control(VoiceControl::InjectFact {
            fact: VerifiedVoiceFact {
                output_generation: Some(1),
                work_context: None,
                speech_source: None,
                correlation_id: "stale".into(),
                upstream_trigger_id: None,
                canonical_receipt_id: "old".into(),
                content: "late fact".into(),
                speak: true,
            },
        })
        .unwrap();
    loop {
        if let VoiceModelEvent::ControlRejected { error } = next_event(&mut session).await {
            assert_eq!(error.kind, VoiceErrorKind::StaleEpoch);
            break;
        }
    }
    assert!(
        session
            .shutdown(VoiceCloseReason::UserEnded)
            .await
            .finalization_confirmed
    );
    server.await.unwrap();
}

#[tokio::test]
async fn stepfun_pending_fact_response_rebuilds_instead_of_relabelling_late_audio() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (response_requested_tx, response_requested_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut response_requested_tx = Some(response_requested_tx);
        for attempt in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert_eq!(next_json(&mut socket).await["type"], "session.update");
            send(&mut socket, json!({"type":"session.updated","session":{"id":format!("step-{attempt}"),"model":"stepaudio-3-realtime-preview","input_audio_format":"pcm16","output_audio_format":"pcm16"}})).await;
            if attempt == 0 {
                send(&mut socket, json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"committed-user","transcript":"Prepare the report"})).await;
                assert_eq!(next_json(&mut socket).await["item"]["role"], "system");
                assert_eq!(next_json(&mut socket).await["type"], "response.create");
                let _ = response_requested_tx.take().unwrap().send(());
                while let Some(message) = socket.next().await {
                    if matches!(message.unwrap(), Message::Close(_)) {
                        // The old session never supplied response.created.
                        // Its close acknowledgment proves the only safe fence.
                        let _ = socket.flush().await;
                        break;
                    }
                }
            } else {
                let user = next_json(&mut socket).await;
                send(
                    &mut socket,
                    json!({"type":"conversation.item.created","item":user["item"]}),
                )
                .await;
                let fact = next_json(&mut socket).await;
                send(
                    &mut socket,
                    json!({"type":"conversation.item.created","item":fact["item"]}),
                )
                .await;
                assert_eq!(user["item"]["role"], "user");
                assert_eq!(fact["item"]["role"], "system");
                assert!(
                    fact["item"]["content"][0]["text"]
                        .as_str()
                        .unwrap()
                        .contains("canonical")
                );
                send(
                    &mut socket,
                    json!({"type":"response.created","response":{"id":"fresh-step-response"}}),
                )
                .await;
                send(&mut socket, json!({"type":"response.audio.delta","response_id":"fresh-step-response","delta":"AAA="})).await;
                while let Some(message) = socket.next().await {
                    if matches!(message.unwrap(), Message::Close(_)) {
                        let _ = socket.flush().await;
                        break;
                    }
                }
            }
        }
    });
    let model = super::create_stepfun_model_port(
        connection(port),
        json!({}),
        "stepaudio-3-realtime-preview".into(),
    )
    .unwrap();
    let mut session = model
        .open(
            request(
                "stepaudio-3-realtime-preview",
                VoiceTransportPreference::Relay,
            ),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(3),
        )
        .await
        .unwrap();
    loop {
        if matches!(
            next_event(&mut session).await,
            VoiceModelEvent::Transcript { .. }
        ) {
            break;
        }
    }
    session
        .input
        .try_control(VoiceControl::InjectFact {
            fact: VerifiedVoiceFact {
                output_generation: Some(1),
                work_context: None,
                speech_source: None,
                correlation_id: "revoked-fact-speech".into(),
                upstream_trigger_id: None,
                canonical_receipt_id: "canonical-receipt".into(),
                content: "The canonical work is running.".into(),
                speak: true,
            },
        })
        .unwrap();
    response_requested_rx.await.unwrap();
    session
        .input
        .try_control(VoiceControl::InterruptOutput {
            output_generation: 2,
            played: None,
        })
        .unwrap();
    let mut boundary = false;
    loop {
        match next_event(&mut session).await {
            VoiceModelEvent::OutputBoundary {
                output_generation: 2,
                ..
            } => boundary = true,
            VoiceModelEvent::OutputStarted { segment } => {
                if segment.output_generation == 2 {
                    assert!(
                        segment.correlation_id.is_none(),
                        "revoked requested speech cannot certify a new response"
                    );
                }
            }
            VoiceModelEvent::Audio { segment_id, frame } => {
                assert!(boundary);
                assert_eq!(frame.output_generation, 2);
                assert_eq!(segment_id, "fresh-step-response");
                break;
            }
            _ => {}
        }
    }
    assert!(
        session
            .shutdown(VoiceCloseReason::UserEnded)
            .await
            .finalization_confirmed
    );
    server.await.unwrap();
}

#[tokio::test]
async fn live_production_port_delegation_and_fresh_relay_output_boundary_contract() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (fact_seen_tx, fact_seen_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut fact_seen_tx = Some(fact_seen_tx);
        for attempt in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let startup = next_json(&mut socket).await;
            assert_eq!(startup["type"], "session.start");
            assert_eq!(startup["session"]["delegation"]["type"], "client");
            assert_eq!(
                startup["session"]["input"],
                json!([]),
                "mutable data must not be baked into immutable startup input/instructions"
            );
            send(&mut socket,json!({"type":"session.started","session":{"id":format!("live-{attempt}"),"model":"gpt-live-1","audio":{"format":{"type":"audio/pcm","rate":24000}},"delegation":{"type":"client"}}})).await;
            if attempt == 0 {
                send(&mut socket,json!({"type":"session.input_transcript.delta","event_id":"input-a","delta":"Run the report","start_ms":0,"end_ms":1000})).await;
                send(&mut socket,json!({"type":"session.delegation.created","offset_ms":1000,"delegation":{"id":"delegation-a","target":"client"}})).await;
                let update = next_json(&mut socket).await;
                assert_eq!(update["type"], "session.thinking.append");
                assert_eq!(update["delegation_id"], "delegation-a");
                send(&mut socket,json!({"type":"session.thinking.appended","client_event_id":update["event_id"],"start_ms":1100,"end_ms":1200})).await;
                let _ = fact_seen_tx.take().unwrap().send(());
            } else {
                let context = ack_live_initialization(&mut socket).await.join("\n");
                assert!(
                    context.contains("The canonical work was admitted.")
                        && context.contains("Committed historical input")
                );
                send(
                    &mut socket,
                    json!({"type":"session.output_audio.delta","delta":"AAA="}),
                )
                .await;
            }
            assert_eq!(next_json(&mut socket).await["type"], "session.close");
            send(
                &mut socket,
                json!({"type":"session.closed","reason":"close_requested","usage":{"seconds":1}}),
            )
            .await;
            let _ = socket.close(None).await;
        }
    });
    let model =
        super::create_openai_live_model_port(connection(port), json!({}), "gpt-live-1".into())
            .unwrap();
    let mut session = model
        .open(
            request("gpt-live-1", VoiceTransportPreference::Relay),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(3),
        )
        .await
        .unwrap();
    loop {
        if let VoiceModelEvent::WorkTrigger { trigger } = next_event(&mut session).await {
            assert!(
                matches!(trigger,WorkTrigger::DelegationTrigger{upstream_trigger_id,offset_ms:1000,..} if upstream_trigger_id=="delegation-a")
            );
            break;
        }
    }
    session
        .input
        .try_control(VoiceControl::InjectFact {
            fact: VerifiedVoiceFact {
                output_generation: None,
                work_context: Some(VoiceWorkContextFact { target: None }),
                speech_source: None,
                correlation_id: "work".into(),
                upstream_trigger_id: Some("delegation-a".into()),
                canonical_receipt_id: "receipt".into(),
                content: "The canonical work was admitted.".into(),
                speak: false,
            },
        })
        .unwrap();
    fact_seen_rx.await.unwrap();
    loop {
        if let VoiceModelEvent::WorkContextCheckpoint {
            context_window_ref,
            media_range,
            canonical_receipt_id,
            ..
        } = next_event(&mut session).await
        {
            assert_eq!(context_window_ref, "live-0");
            assert_eq!(media_range.end_us, 1_200_000);
            assert_eq!(canonical_receipt_id, "receipt");
            break;
        }
    }
    session
        .input
        .try_control(VoiceControl::InterruptOutput {
            output_generation: 2,
            played: None,
        })
        .unwrap();
    let mut boundary = false;
    loop {
        match next_event(&mut session).await {
            VoiceModelEvent::OutputBoundary {
                output_generation: 2,
                ..
            } => boundary = true,
            VoiceModelEvent::Audio { frame, .. } => {
                assert!(boundary);
                assert_eq!(frame.output_generation, 2);
                break;
            }
            _ => {}
        }
    }
    assert!(
        session
            .shutdown(VoiceCloseReason::UserEnded)
            .await
            .finalization_confirmed
    );
    server.await.unwrap();
}
