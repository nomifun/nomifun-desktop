//! Production relay protocol and private cache revocation; no cloud/device claim.
use super::contract_tests::{
    ack_live_initialization, connection, next_event, next_json, request, send,
};
use nomifun_voice_contracts::voice::*;
use serde_json::json;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

fn source(id: &str, revision: u64, through_seq: u64) -> VoiceSpeechSourceRef {
    VoiceSpeechSourceRef {
        message_id: id.into(),
        revision,
        through_seq,
    }
}
fn fact(
    content: &str,
    speech_source: VoiceSpeechSourceRef,
    generation: u64,
    speak: bool,
) -> VoiceControl {
    VoiceControl::InjectFact {
        fact: VerifiedVoiceFact {
            correlation_id: format!("speech:{}", speech_source.message_id),
            upstream_trigger_id: None,
            canonical_receipt_id: "canonical-source-receipt".into(),
            content: content.into(),
            speak,
            output_generation: Some(generation),
            work_context: None,
            speech_source: Some(speech_source),
        },
    }
}

#[tokio::test]
async fn two_production_ports_wait_for_real_initial_fact_ack_before_open_returns() {
    for step in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        let gate = std::sync::Arc::new(tokio::sync::Notify::new());
        let server_gate = gate.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let startup = next_json(&mut socket).await;
            let serialized = startup.to_string();
            assert!(!serialized.contains("INITIAL_ASSISTANT_BODY"));
            if step {
                assert_eq!(startup["type"], "session.update");
                send(&mut socket,json!({"type":"session.updated","session":{"id":"initial-step","model":"stepaudio-3-realtime-preview","input_audio_format":"pcm16","output_audio_format":"pcm16"}})).await;
                let item = next_json(&mut socket).await;
                assert_eq!(item["type"], "conversation.item.create");
                assert!(
                    item["item"]["content"][0]["text"]
                        .as_str()
                        .unwrap()
                        .contains("INITIAL_ASSISTANT_BODY")
                );
                send(&mut socket,json!({"type":"conversation.item.created","item":{"type":"message","role":"system","content":[{"type":"input_text","text":"unrelated context"}]}})).await;
                let _ = seen_tx.send(());
                server_gate.notified().await;
                send(
                    &mut socket,
                    json!({"type":"conversation.item.created","item":item["item"]}),
                )
                .await;
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
                assert_eq!(startup["type"], "session.start");
                assert_eq!(startup["session"]["input"], json!([]));
                send(&mut socket,json!({"type":"session.started","session":{"id":"initial-live","model":"gpt-live-1","audio":{"format":{"type":"audio/pcm","rate":24000}},"delegation":{"type":"client"}}})).await;
                let initial = next_json(&mut socket).await;
                assert_eq!(initial["type"], "session.thinking.append");
                assert!(
                    initial["content"]
                        .as_str()
                        .unwrap()
                        .contains("INITIAL_ASSISTANT_BODY")
                );
                send(&mut socket,json!({"type":"session.thinking.appended","client_event_id":"unrelated","start_ms":0,"end_ms":0})).await;
                let _ = seen_tx.send(());
                server_gate.notified().await;
                send(&mut socket,json!({"type":"session.thinking.appended","client_event_id":initial["event_id"],"start_ms":0,"end_ms":0})).await;
                ack_live_initialization(&mut socket).await;
                assert_eq!(next_json(&mut socket).await["type"], "session.close");
                send(
                    &mut socket,
                    json!({"type":"session.closed","reason":"close_requested"}),
                )
                .await;
                let _ = socket.close(None).await;
            }
        });
        let model_name = if step {
            "stepaudio-3-realtime-preview"
        } else {
            "gpt-live-1"
        };
        let model = if step {
            super::create_stepfun_model_port(connection(port), json!({}), model_name.into())
        } else {
            super::create_openai_live_model_port(connection(port), json!({}), model_name.into())
        }
        .unwrap();
        let mut open = request(model_name, VoiceTransportPreference::Relay);
        open.instructions = "static policy".into();
        open.initial_facts.push(VerifiedVoiceFact {
            correlation_id: "initial".into(),
            upstream_trigger_id: None,
            canonical_receipt_id: "event-10".into(),
            content: "INITIAL_ASSISTANT_BODY".into(),
            speak: false,
            output_generation: Some(1),
            work_context: None,
            speech_source: Some(source("initial-message", 1, 10)),
        });
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
            "configuration ACK/unrelated ACK does not complete source initialization"
        );
        gate.notify_one();
        let session = opening.await.unwrap().unwrap();
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
async fn live_revoke_before_interrupt_rebuild_filters_old_body_and_rejects_late_source() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let (facts_seen_tx, facts_seen_rx) = tokio::sync::oneshot::channel();
    let (replacement_seen_tx, replacement_seen_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let mut facts_seen_tx = Some(facts_seen_tx);
        let mut replacement_seen_tx = Some(replacement_seen_tx);
        for attempt in 0..2 {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let startup = next_json(&mut socket).await;
            assert_eq!(startup["type"], "session.start");
            assert_eq!(startup["session"]["input"], json!([]));
            send(&mut socket,json!({"type":"session.started","session":{"id":format!("source-live-{attempt}"),"model":"gpt-live-1","audio":{"format":{"type":"audio/pcm","rate":24000}},"delegation":{"type":"client"}}})).await;
            if attempt == 1 {
                let restored = ack_live_initialization(&mut socket).await.join("\n");
                assert!(
                    !restored.contains("INVALID_OLD_SPEECH_BODY"),
                    "rebuild must not restore revoked canonical speech wording"
                );
                assert!(restored.contains("VALID_OTHER_SPEECH_BODY"));
                assert!(
                    restored.contains("media-consumption checkpoint")
                        && restored.contains("unknown"),
                    "retain reported timing without claiming heard words"
                );
                assert!(
                    !restored.contains("speech_source"),
                    "private cache keys are not supplier input fields"
                );
            }
            if attempt == 0 {
                let old = next_json(&mut socket).await;
                assert_eq!(old["type"], "session.commentary.append");
                assert_eq!(old["content"], "INVALID_OLD_SPEECH_BODY");
                assert!(old.get("speech_source").is_none());
                let valid = next_json(&mut socket).await;
                assert_eq!(valid["type"], "session.thinking.append");
                assert_eq!(valid["content"], "VALID_OTHER_SPEECH_BODY");
                let _ = facts_seen_tx.take().unwrap().send(());
                let revoke = next_json(&mut socket).await;
                assert_eq!(revoke["type"], "session.instructions.append");
                assert!(
                    revoke["content"].as_str().unwrap().contains("revoked")
                        && revoke["content"].as_str().unwrap().contains("old-message")
                );
                assert!(revoke.get("speech_source").is_none());
            } else {
                let replacement = next_json(&mut socket).await;
                assert_eq!(replacement["type"], "session.commentary.append");
                assert_eq!(
                    replacement["content"], "VALID_REPLACEMENT_BODY",
                    "late revoked InjectFact must not produce any wire command"
                );
                let _ = replacement_seen_tx.take().unwrap().send(());
            }
            assert_eq!(next_json(&mut socket).await["type"], "session.close");
            send(
                &mut socket,
                json!({"type":"session.closed","reason":"close_requested"}),
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
    session
        .input
        .try_control(fact(
            "INVALID_OLD_SPEECH_BODY",
            source("old-message", 1, 10),
            1,
            true,
        ))
        .unwrap();
    session
        .input
        .try_control(fact(
            "VALID_OTHER_SPEECH_BODY",
            source("other-message", 1, 11),
            1,
            false,
        ))
        .unwrap();
    facts_seen_rx.await.unwrap();
    session
        .input
        .try_control(VoiceControl::RevokeSpeech {
            source: source("old-message", 2, 20),
            output_generation: 2,
        })
        .unwrap();
    session
        .input
        .try_control(VoiceControl::InterruptOutput {
            output_generation: 2,
            played: Some(PlaybackReceipt {
                activation_epoch: 1,
                output_generation: 1,
                segment_id: "reported-old-output".into(),
                revision: 1,
                state: DeliveryState::Played,
                consumed_us: 20_000,
                uncertain_tail_us: 10_000,
                precision: PlaybackPrecision::Unknown,
            }),
        })
        .unwrap();
    loop {
        if matches!(
            next_event(&mut session).await,
            VoiceModelEvent::OutputBoundary {
                output_generation: 2,
                ..
            }
        ) {
            break;
        }
    }
    session
        .input
        .try_control(fact(
            "LATE_INVALID_OLD_SPEECH_BODY",
            source("old-message", 1, 10),
            2,
            true,
        ))
        .unwrap();
    loop {
        if let VoiceModelEvent::ControlRejected { error } = next_event(&mut session).await {
            assert_eq!(error.kind, VoiceErrorKind::StaleEpoch);
            assert!(error.message.contains("source was revoked"));
            break;
        }
    }
    session
        .input
        .try_control(fact(
            "VALID_REPLACEMENT_BODY",
            source("old-message", 3, 30),
            2,
            true,
        ))
        .unwrap();
    replacement_seen_rx.await.unwrap();
    assert!(
        session
            .shutdown(VoiceCloseReason::UserEnded)
            .await
            .finalization_confirmed
    );
    server.await.unwrap();
}
