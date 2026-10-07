//! Real local WS production actors: ACK identity, media gate and close lane.
//! This is protocol validation and does not certify a cloud/device experience.
use super::contract_tests::{connection, next_event, next_json, request, send};
use nomifun_voice_contracts::voice::*;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

fn frame(sequence: u64) -> AudioFrame {
    AudioFrame {
        activation_epoch: 1,
        output_generation: 1,
        sequence,
        timestamp: sequence * 20_000,
        duration_us: 20_000,
        format: AudioFormat::pcm16(24000, 1),
        payload: vec![0; 960],
    }
}
async fn started<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    step: bool,
) {
    let startup = next_json(socket).await;
    if step {
        assert_eq!(startup["type"], "session.update");
        send(socket,json!({"type":"session.updated","session":{"id":"input-step","model":"stepaudio-3-realtime-preview","input_audio_format":"pcm16","output_audio_format":"pcm16"}})).await;
    } else {
        assert_eq!(startup["type"], "session.start");
        send(socket,json!({"type":"session.started","session":{"id":"input-live","model":"gpt-live-1","audio":{"format":{"type":"audio/pcm","rate":24000}},"delegation":{"type":"client"}}})).await;
    }
}
async fn close_server<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
    socket: &mut tokio_tungstenite::WebSocketStream<S>,
    step: bool,
) {
    if step {
        use futures_util::{SinkExt, StreamExt};
        while let Some(message) = socket.next().await {
            if matches!(
                message.unwrap(),
                tokio_tungstenite::tungstenite::Message::Close(_)
            ) {
                let _ = socket.flush().await;
                break;
            }
        }
    } else {
        assert_eq!(next_json(socket).await["type"], "session.close");
        send(
            socket,
            json!({"type":"session.closed","reason":"close_requested"}),
        )
        .await;
        let _ = socket.close(None).await;
    }
}

#[tokio::test]
async fn production_input_controls_wait_for_exact_ack_and_never_forward_early_resume_audio() {
    for step in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let gate = Arc::new(tokio::sync::Notify::new());
        let server_gate = gate.clone();
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
        let (audio_tx, audio_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            started(&mut socket, step).await;
            if step {
                assert_eq!(
                    next_json(&mut socket).await["type"],
                    "input_audio_buffer.clear"
                );
            }
            let pause = next_json(&mut socket).await;
            assert_eq!(
                pause["type"],
                if step {
                    "input_audio_buffer.clear"
                } else {
                    "session.input_audio.mute"
                }
            );
            // Step's documented clear ACK has no client reference. The first
            // ACK here belongs to the preceding legacy clear in actor FIFO,
            // never to the product request. Live supplies a real client ID.
            if step {
                send(
                    &mut socket,
                    json!({"type":"input_audio_buffer.cleared","event_id":"legacy-clear-ack"}),
                )
                .await;
            } else {
                send(
                    &mut socket,
                    json!({"type":"session.input_audio.muted","client_event_id":"wrong-request"}),
                )
                .await;
            }
            let _ = seen_tx.send(());
            server_gate.notified().await;
            if step {
                send(
                    &mut socket,
                    json!({"type":"input_audio_buffer.cleared","event_id":"product-clear-ack"}),
                )
                .await;
            } else {
                send(
                    &mut socket,
                    json!({"type":"session.input_audio.muted","client_event_id":pause["event_id"]}),
                )
                .await;
            }
            let resume = next_json(&mut socket).await;
            assert_eq!(
                resume["type"],
                if step {
                    "input_audio_buffer.clear"
                } else {
                    "session.input_audio.unmute"
                }
            );
            let _ = resume_tx.send(());
            assert!(
                tokio::time::timeout(Duration::from_millis(60), next_json(&mut socket))
                    .await
                    .is_err(),
                "audio sent during pending unmute must stay behind the input gate"
            );
            if step {
                send(
                    &mut socket,
                    json!({"type":"input_audio_buffer.cleared","event_id":"resume-clear-ack"}),
                )
                .await;
            } else {
                send(&mut socket,json!({"type":"session.input_audio.unmuted","client_event_id":resume["event_id"]})).await;
            }
            assert_eq!(
                next_json(&mut socket).await["type"],
                if step {
                    "input_audio_buffer.append"
                } else {
                    "session.input_audio.append"
                }
            );
            let _ = audio_tx.send(());
            close_server(&mut socket, step).await;
        });
        let name = if step {
            "stepaudio-3-realtime-preview"
        } else {
            "gpt-live-1"
        };
        let model = if step {
            super::create_stepfun_model_port(connection(port), json!({}), name.into())
        } else {
            super::create_openai_live_model_port(connection(port), json!({}), name.into())
        }
        .unwrap();
        let mut session = model
            .open(
                request(name, VoiceTransportPreference::Relay),
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(3),
            )
            .await
            .unwrap();
        assert!(matches!(
            next_event(&mut session).await,
            VoiceModelEvent::Ready { .. }
        ));
        assert!(matches!(
            next_event(&mut session).await,
            VoiceModelEvent::ContextOpened { .. }
        ));
        if step {
            session
                .input
                .try_control(VoiceControl::MuteInput { muted: true })
                .unwrap();
        }
        session
            .input
            .try_control(VoiceControl::SetInputMuted {
                muted: true,
                request_id: "product-pause".into(),
            })
            .unwrap();
        seen_rx.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(40), session.events.recv())
                .await
                .is_err(),
            "wrong or preceding ACK does not certify the product pause"
        );
        gate.notify_one();
        assert!(
            matches!(next_event(&mut session).await,VoiceModelEvent::InputMuteApplied{muted:true,request_id} if request_id=="product-pause")
        );
        session
            .input
            .try_control(VoiceControl::SetInputMuted {
                muted: false,
                request_id: "product-resume".into(),
            })
            .unwrap();
        resume_rx.await.unwrap();
        session.input.try_audio(frame(1)).unwrap();
        assert!(
            matches!(next_event(&mut session).await,VoiceModelEvent::InputMuteApplied{muted:false,request_id} if request_id=="product-resume")
        );
        session.input.try_audio(frame(2)).unwrap();
        audio_rx.await.unwrap();
        assert!(
            session
                .shutdown(VoiceCloseReason::UserEnded)
                .await
                .finalization_confirmed
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn production_close_bypasses_a_pending_input_ack() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        started(&mut socket, false).await;
        assert_eq!(
            next_json(&mut socket).await["type"],
            "session.input_audio.mute"
        );
        let _ = seen_tx.send(());
        close_server(&mut socket, false).await;
    });
    let model =
        super::create_openai_live_model_port(connection(port), json!({}), "gpt-live-1".into())
            .unwrap();
    let session = model
        .open(
            request("gpt-live-1", VoiceTransportPreference::Relay),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(3),
        )
        .await
        .unwrap();
    session
        .input
        .try_control(VoiceControl::SetInputMuted {
            muted: true,
            request_id: "never-acknowledged".into(),
        })
        .unwrap();
    seen_rx.await.unwrap();
    let close = tokio::time::timeout(
        Duration::from_secs(1),
        session.shutdown(VoiceCloseReason::UserEnded),
    )
    .await
    .unwrap();
    assert!(close.finalization_confirmed);
    server.await.unwrap();
}

#[tokio::test]
async fn production_input_ack_deadline_fails_voice_without_fabricating_applied() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        started(&mut socket, false).await;
        assert_eq!(
            next_json(&mut socket).await["type"],
            "session.input_audio.unmute"
        );
        use futures_util::StreamExt;
        while let Some(message) = socket.next().await {
            if matches!(
                message.unwrap(),
                tokio_tungstenite::tungstenite::Message::Close(_)
            ) {
                break;
            }
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
    session
        .input
        .try_control(VoiceControl::SetInputMuted {
            muted: false,
            request_id: "missing-ack".into(),
        })
        .unwrap();
    loop {
        match next_event(&mut session).await {
            VoiceModelEvent::InputMuteApplied { .. } => {
                panic!("socket write did not prove input acceptance")
            }
            VoiceModelEvent::ControlRejected { error } => {
                assert_eq!(error.kind, VoiceErrorKind::Deadline);
                break;
            }
            _ => {}
        }
    }
    let close = session.shutdown(VoiceCloseReason::UserEnded).await;
    assert_eq!(close.reason, VoiceCloseReason::ProviderFailed);
    assert!(!close.finalization_confirmed);
    server.await.unwrap();
}

#[tokio::test]
async fn relay_recovery_announces_pause_discards_tail_and_requires_a_new_input_ack() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (startup_tx, startup_rx) = tokio::sync::oneshot::channel();
    let gate = Arc::new(tokio::sync::Notify::new());
    let server_gate = gate.clone();
    let (audio_tx, audio_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut first = tokio_tungstenite::accept_async(stream).await.unwrap();
        started(&mut first, false).await;
        close_server(&mut first, false).await;
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        assert_eq!(next_json(&mut socket).await["type"], "session.start");
        let _ = startup_tx.send(());
        server_gate.notified().await;
        send(&mut socket,json!({"type":"session.started","session":{"id":"recovered-live","model":"gpt-live-1","audio":{"format":{"type":"audio/pcm","rate":24000}},"delegation":{"type":"client"}}})).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(60), next_json(&mut socket))
                .await
                .is_err(),
            "recovered transport must not silently reopen capture or replay a buffered tail"
        );
        let unmute = next_json(&mut socket).await;
        assert_eq!(unmute["type"], "session.input_audio.unmute");
        send(
            &mut socket,
            json!({"type":"session.input_audio.unmuted","client_event_id":unmute["event_id"]}),
        )
        .await;
        assert_eq!(
            next_json(&mut socket).await["type"],
            "session.input_audio.append"
        );
        let _ = audio_tx.send(());
        close_server(&mut socket, false).await;
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
    session
        .input
        .try_control(VoiceControl::InterruptOutput {
            output_generation: 2,
            played: None,
        })
        .unwrap();
    loop {
        if matches!(
            next_event(&mut session).await,
            VoiceModelEvent::RelayRecovering {
                output_generation: 2
            }
        ) {
            break;
        }
    }
    startup_rx.await.unwrap();
    let mut tail = frame(1);
    tail.output_generation = 2;
    session.input.try_audio(tail).unwrap();
    tokio::time::sleep(Duration::from_millis(260)).await;
    gate.notify_one();
    loop {
        if matches!(
            next_event(&mut session).await,
            VoiceModelEvent::RelayRecovered {
                output_generation: 2
            }
        ) {
            break;
        }
    }
    let mut early = frame(2);
    early.output_generation = 2;
    session.input.try_audio(early).unwrap();
    tokio::time::sleep(Duration::from_millis(90)).await;
    session
        .input
        .try_control(VoiceControl::SetInputMuted {
            muted: false,
            request_id: "fresh-connection-resume".into(),
        })
        .unwrap();
    loop {
        if matches!(next_event(&mut session).await,VoiceModelEvent::InputMuteApplied{muted:false,request_id} if request_id=="fresh-connection-resume")
        {
            break;
        }
    }
    let mut fresh = frame(3);
    fresh.output_generation = 2;
    session.input.try_audio(fresh).unwrap();
    audio_rx.await.unwrap();
    assert!(
        session
            .shutdown(VoiceCloseReason::UserEnded)
            .await
            .finalization_confirmed
    );
    server.await.unwrap();
}

#[tokio::test]
async fn both_production_ports_publish_current_local_sources_silently_and_require_real_ack() {
    for step in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (acked_tx, acked_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            started(&mut socket, step).await;
            let transcript = String::from("WHOLE_USER_INPUT")
                + &"x".repeat(32 * 1024 - "WHOLE_USER_INPUT".len());
            if step {
                send(&mut socket,json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"source-user","transcript":"WHOLE_USER_INPUT"})).await;
            } else {
                send(&mut socket,json!({"type":"session.input_transcript.delta","event_id":"source-user","delta":transcript,"start_ms":10,"end_ms":1200000})).await;
            }
            let mut contents = String::new();
            if step {
                let command = next_json(&mut socket).await;
                assert_eq!(command["type"], "conversation.item.create");
                assert_eq!(command["item"]["role"], "system");
                contents = command["item"]["content"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_owned();
                send(
                    &mut socket,
                    json!({"type":"conversation.item.created","item":command["item"]}),
                )
                .await;
            } else {
                loop {
                    let command = next_json(&mut socket).await;
                    assert_eq!(command["type"], "session.thinking.append");
                    assert!(
                        command.get("source_ref").is_none(),
                        "application data must not become unknown supplier wire fields"
                    );
                    contents.push_str(command["content"].as_str().unwrap());
                    send(&mut socket,json!({"type":"session.thinking.appended","client_event_id":command["event_id"],"start_ms":40,"end_ms":40})).await;
                    if contents
                        .split_once("Transcript data: ")
                        .is_some_and(|(_, json)| {
                            serde_json::from_str::<TranscriptFragment>(json).is_ok()
                        })
                    {
                        break;
                    }
                }
            }
            assert!(
                contents.contains("WHOLE_USER_INPUT")
                    && contents.contains("fragment_id")
                    && contents.contains("revision")
            );
            assert!(!contents.contains("canonical_receipt_id"));
            assert!(
                contents.contains("not a new user message")
                    && contents.contains("ASR delivery time does not identify")
            );
            let _ = acked_tx.send(());
            close_server(&mut socket, step).await;
        });
        let name = if step {
            "stepaudio-3-realtime-preview"
        } else {
            "gpt-live-1"
        };
        let model = if step {
            super::create_stepfun_model_port(connection(port), json!({}), name.into())
        } else {
            super::create_openai_live_model_port(connection(port), json!({}), name.into())
        }
        .unwrap();
        let mut session = model
            .open(
                request(name, VoiceTransportPreference::Relay),
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(3),
            )
            .await
            .unwrap();
        let fragment = loop {
            if let VoiceModelEvent::Transcript { fragment } = next_event(&mut session).await {
                break fragment;
            }
        };
        session
            .input
            .try_control(VoiceControl::PresentInputSource { fragment })
            .unwrap();
        acked_rx.await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(2200), session.events.recv())
                .await
                .is_err(),
            "only the real complete ACK clears the source publication deadline; no response or work trigger is synthesized"
        );
        assert!(
            session
                .shutdown(VoiceCloseReason::UserEnded)
                .await
                .finalization_confirmed
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn production_live_receipt_control_ack_burst_remains_bounded_and_close_stays_priority() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (acked_tx, acked_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
        started(&mut socket, false).await;
        let mut contents = String::new();
        let mut count = 0;
        loop {
            let command = next_json(&mut socket).await;
            assert_eq!(command["type"], "session.thinking.append");
            contents.push_str(command["content"].as_str().unwrap());
            count += 1;
            send(&mut socket,json!({"type":"session.thinking.appended","client_event_id":command["event_id"],"start_ms":50,"end_ms":51})).await;
            if contents.len() == 4096 {
                break;
            }
        }
        assert!(
            count > 4,
            "the production control burst exceeds the old slot limit"
        );
        let _ = acked_tx.send(());
        close_server(&mut socket, false).await;
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
    session
        .input
        .try_control(VoiceControl::InjectFact {
            fact: VerifiedVoiceFact {
                correlation_id: "receipt-fixture".into(),
                canonical_receipt_id: "original-canonical-reference".into(),
                upstream_trigger_id: None,
                content: "x".repeat(4096),
                speak: false,
                output_generation: Some(1),
                work_context: None,
                speech_source: None,
            },
        })
        .unwrap();
    acked_rx.await.unwrap();
    loop {
        match tokio::time::timeout(Duration::from_millis(60), session.events.recv()).await {
            Err(_) => break,
            Ok(Some(VoiceModelEvent::Ready { .. } | VoiceModelEvent::ContextOpened { .. })) => {}
            other => panic!("healthy control ACK burst failed: {other:?}"),
        }
    }
    let termination = tokio::time::timeout(
        Duration::from_secs(1),
        session.shutdown(VoiceCloseReason::UserEnded),
    )
    .await
    .unwrap();
    assert!(termination.finalization_confirmed);
    server.await.unwrap();
}
