//! Voice-local recovery through production wire ports; no cloud/UX claim.
use super::contract_tests::{
    ack_live_initialization, connection, next_event, next_json, request, send,
};
use nomifun_voice_contracts::voice::*;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(super) fn replay(scope: VoiceReplayScope) -> VoiceLocalReplay {
    let origin = |sequence| VoiceLocalFactRef {
        voice_session_id: "prior-voice".into(),
        activation_epoch: 3,
        journal_sequence: sequence,
    };
    VoiceLocalReplay {
        scope,
        entries: vec![
            VoiceLocalReplayEntry::UserCommitted {
                origin: origin(5),
                fragment: TranscriptFragment {
                    speaker: VoiceSpeaker::User,
                    fragment_id: "prior-user-fragment".into(),
                    revision: 2,
                    commit: TranscriptCommit::Committed,
                    text: "PRIOR_COMMITTED_USER_INPUT".into(),
                    media_range: Some(MediaRange {
                        start_us: 10_000,
                        end_us: 30_000,
                    }),
                },
            },
            VoiceLocalReplayEntry::Playback {
                origin: origin(8),
                receipt: PlaybackReceipt {
                    activation_epoch: 3,
                    output_generation: 4,
                    segment_id: "prior-segment".into(),
                    revision: 1,
                    state: DeliveryState::Interrupted,
                    consumed_us: 20_000,
                    uncertain_tail_us: 10_000,
                    precision: PlaybackPrecision::Unknown,
                },
            },
        ],
    }
}

#[tokio::test]
async fn replay_binding_or_context_floor_mismatch_rejects_before_network() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let model = super::create_stepfun_model_port(
        connection(listener.local_addr().unwrap().port()),
        json!({}),
        "stepaudio-3-realtime-preview".into(),
    )
    .unwrap();
    for binding_mismatch in [false, true] {
        let mut open = request(
            "stepaudio-3-realtime-preview",
            VoiceTransportPreference::Relay,
        );
        let mut stale = open.replay_scope.clone();
        if binding_mismatch {
            stale.binding_version += 1;
        } else {
            stale.context_floor += 1;
        }
        open.initial_local_replay = Some(replay(stale));
        let error = match model
            .open(
                open,
                CancellationToken::new(),
                Instant::now() + Duration::from_secs(2),
            )
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("cross-scope local history was accepted"),
        };
        assert_eq!(error.kind, VoiceErrorKind::StaleBinding);
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err(),
        "scope rejection must precede any upstream connection"
    );
}

#[tokio::test]
async fn both_production_relays_restore_only_user_and_consumption_records_with_real_ack() {
    for step in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let gate = Arc::new(tokio::sync::Notify::new());
        let server_gate = gate.clone();
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let startup = next_json(&mut socket).await;
            assert!(!startup.to_string().contains("PRIOR_COMMITTED_USER_INPUT"));
            assert!(!startup.to_string().contains("prior-segment"));
            let restored = if step {
                send(&mut socket,json!({"type":"session.updated","session":{"id":"replay-step","model":"stepaudio-3-realtime-preview","input_audio_format":"pcm16","output_audio_format":"pcm16"}})).await;
                let user = next_json(&mut socket).await;
                assert_eq!(user["type"], "conversation.item.create");
                assert_eq!(user["item"]["role"], "user");
                send(
                    &mut socket,
                    json!({"type":"conversation.item.created","item":user["item"]}),
                )
                .await;
                let playback = next_json(&mut socket).await;
                assert_eq!(playback["type"], "conversation.item.create");
                assert_eq!(playback["item"]["role"], "system");
                // A stale tool event during silent restoration is not current user intent.
                send(&mut socket,json!({"type":"response.function_call_arguments.done","call_id":"historical-tool","name":"start","arguments":"{\"text\":\"old task\"}"})).await;
                let _ = seen_tx.send(());
                server_gate.notified().await;
                send(
                    &mut socket,
                    json!({"type":"conversation.item.created","item":playback["item"]}),
                )
                .await;
                format!("{} {}", user["item"], playback["item"])
            } else {
                assert_eq!(startup["session"]["input"], json!([]));
                send(&mut socket,json!({"type":"session.started","session":{"id":"replay-live","model":"gpt-live-1","audio":{"format":{"type":"audio/pcm","rate":24000}},"delegation":{"type":"client"}}})).await;
                let first = next_json(&mut socket).await;
                assert_eq!(first["type"], "session.thinking.append");
                assert!(
                    first.get("origin").is_none() && first.get("canonical_receipt_id").is_none()
                );
                send(&mut socket,json!({"type":"delegation.started","delegation_id":"historical-delegation","target":"work","offset_ms":0})).await;
                let _ = seen_tx.send(());
                server_gate.notified().await;
                send(&mut socket,json!({"type":"session.thinking.appended","client_event_id":first["event_id"],"start_ms":0,"end_ms":0})).await;
                format!(
                    "{} {}",
                    first["content"].as_str().unwrap(),
                    ack_live_initialization(&mut socket).await.join("")
                )
            };
            assert!(restored.contains("PRIOR_COMMITTED_USER_INPUT"));
            assert!(restored.contains("prior-segment"));
            assert!(
                restored.contains("unknown")
                    && restored.contains("heard words and work completion are not inferred")
            );
            assert!(!restored.contains("canonical_receipt_id"));
            if step {
                use futures_util::{SinkExt, StreamExt};
                while let Some(message) = socket.next().await {
                    assert!(
                        matches!(
                            message.unwrap(),
                            tokio_tungstenite::tungstenite::Message::Close(_)
                        ),
                        "restoration must not create a response or append audio"
                    );
                    let _ = socket.flush().await;
                    break;
                }
            } else {
                assert_eq!(next_json(&mut socket).await["type"], "session.close");
                send(
                    &mut socket,
                    json!({"type":"session.closed","reason":"close_requested"}),
                )
                .await;
                let _ = socket.close(None).await;
            }
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
        let mut open = request(name, VoiceTransportPreference::Relay);
        open.instructions = "static voice policy".into();
        open.initial_local_replay = Some(replay(open.replay_scope.clone()));
        let opening = tokio::spawn(async move {
            model
                .open(
                    open,
                    CancellationToken::new(),
                    Instant::now() + Duration::from_secs(3),
                )
                .await
        });
        seen_rx.await.unwrap();
        assert!(
            !opening.is_finished(),
            "restoration must complete its real ACK before Ready"
        );
        gate.notify_one();
        let mut session = opening.await.unwrap().unwrap();
        assert!(matches!(
            next_event(&mut session).await,
            VoiceModelEvent::Ready { .. }
        ));
        assert!(matches!(
            next_event(&mut session).await,
            VoiceModelEvent::ContextOpened { .. }
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(20), session.events.recv())
                .await
                .is_err(),
            "restored historical input must not emit fresh trigger/transcript/audio"
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
