use super::*;
use tokio_tungstenite::accept_async;

fn pcm() -> Vec<i16> {
    (0..960)
        .map(|n| ((n as f64 * 0.1).sin() * 4000.0) as i16)
        .collect()
}

async fn actor(
    deepgram: bool,
    socket: Socket,
) -> (
    StreamingAsrInput,
    mpsc::Receiver<StreamingAsrEvent>,
    tokio::task::JoinHandle<Result<String, String>>,
) {
    let (tx, commands) = mpsc::channel(AUDIO_QUEUE);
    let (events, rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    let input = StreamingAsrInput::new(tx, cancel.clone());
    let finish_requested = input.finish_requested.clone();
    let worker = tokio::spawn(async move {
        run(
            socket,
            if deepgram {
                Wire::Deepgram
            } else {
                Wire::OpenAi
            },
            commands,
            &events,
            cancel,
            finish_requested,
        )
        .await
    });
    (input, rx, worker)
}

#[test]
fn resampling_preserves_phase_and_tail_across_arbitrary_packet_boundaries() {
    let samples: Vec<i16> = (0..1920).map(|n| (n * 13 - 9000) as i16).collect();
    let mut all = audio::Resampler16To24::default();
    let mut expected = all.push(&samples);
    expected.extend(all.finish());
    let mut chunked = audio::Resampler16To24::default();
    let mut actual = Vec::new();
    for chunk in samples.chunks(17) {
        actual.extend(chunked.push(chunk));
    }
    actual.extend(chunked.finish());
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 2880);
    assert!(chunked.finish().is_empty());
}

#[test]
fn transcript_finals_are_idempotent_and_unfinished_text_cannot_be_committed() {
    let mut text = Transcript::default();
    text.put("first".into(), "临时".into(), false).unwrap();
    assert!(text.completed().is_err());
    assert!(text.put("first".into(), "完成🙂".into(), true).unwrap());
    assert!(!text.put("first".into(), "完成🙂".into(), true).unwrap());
    assert!(text.put("first".into(), "迟到的改写".into(), true).is_err());
    text.put("second".into(), "第二段".into(), true).unwrap();
    assert_eq!(text.completed().unwrap(), "完成🙂第二段");
}

#[test]
fn transcript_concatenation_preserves_provider_newlines_and_edge_whitespace() {
    let mut text = Transcript::default();
    let first = "\t \r\n第一行\n";
    let second = "\n第二行🙂 \t\r\n ";
    text.put("first".into(), first.into(), true).unwrap();
    text.put("second".into(), second.into(), false).unwrap();
    let expected = "\t \r\n第一行\n\n第二行🙂 \t\r\n ";
    assert_eq!(text.text(false), expected);
    assert_eq!(text.text(true), first);
    assert!(text.completed().is_err());
    text.put("second".into(), second.into(), true).unwrap();
    assert_eq!(text.completed().unwrap(), expected);
}

#[test]
fn streaming_catalog_protocols_are_distinct_from_upload_protocols() {
    assert!(!supports_protocol("openai.audio_transcriptions"));
    assert!(!supports_protocol("stepfun.asr_sse"));
    for protocol in [
        "deepgram.listen_stream",
        "openai.realtime_transcription",
        "stepfun.asr_stream",
    ] {
        let descriptor =
            crate::protocol_task_descriptor(protocol, ModelTask::SpeechRecognition).unwrap();
        assert_eq!(
            descriptor.transport,
            crate::ProtocolTransportKind::Websocket
        );
        assert_eq!(
            descriptor.executor,
            if protocol == "deepgram.listen_stream" {
                crate::ProtocolExecutorKind::ModelInvoke
            } else {
                crate::ProtocolExecutorKind::RealtimeSession
            }
        );
    }
}

#[tokio::test]
async fn deepgram_receives_pcm_and_returns_partial_before_eos_then_finalizes_once() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let bytes = socket.next().await.unwrap().unwrap().into_data();
        assert_eq!(bytes.len(), 1920); // 960 PCM16, while hardware is recording.
        socket.send(Message::Text(json!({"type":"Results","start":0.0,"is_final":false,"channel":{"alternatives":[{"transcript":"实时字"}]}}).to_string().into())).await.unwrap();
        let command: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(command["type"], "CloseStream");
        socket.send(Message::Text(json!({"type":"Results","start":0.0,"is_final":true,"channel":{"alternatives":[{"transcript":"实时字幕。"}]}}).to_string().into())).await.unwrap();
        socket
            .send(Message::Text(
                json!({"type":"Metadata","duration":0.06})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
    });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .unwrap();
    let (input, mut events, worker) = actor(true, socket).await;
    input.append_pcm16_16k(&pcm()).unwrap();
    assert_eq!(
        events.recv().await.unwrap(),
        StreamingAsrEvent::Partial {
            text: "实时字".into()
        }
    );
    input.finish().await.unwrap();
    input.finish().await.unwrap(); // One physical commit despite repeated intents.
    assert!(input.append_pcm16_16k(&pcm()).is_err());
    assert_eq!(
        events.recv().await.unwrap(),
        StreamingAsrEvent::Final {
            segment_id: "0.000000".into(),
            text: "实时字幕。".into()
        }
    );
    assert_eq!(worker.await.unwrap().unwrap(), "实时字幕。");
    peer.await.unwrap();
}

#[tokio::test]
async fn openai_live_sends_resampled_audio_before_commit_and_preserves_the_tail() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let mut total = 0;
        let mut first = true;
        loop {
            let raw: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            match raw["type"].as_str().unwrap() {
                "input_audio_buffer.append" => {
                    total += base64::engine::general_purpose::STANDARD
                        .decode(raw["audio"].as_str().unwrap())
                        .unwrap()
                        .len();
                    if first {
                        first = false;
                        socket.send(Message::Text(json!({"type":"conversation.item.input_audio_transcription.delta","item_id":"item-1","delta":"尾音"}).to_string().into())).await.unwrap();
                    }
                }
                "input_audio_buffer.commit" => break,
                other => panic!("unexpected command {other}"),
            }
        }
        assert_eq!(total, 2880); // Exactly 60 ms at 24 kHz, including resampler tail.
        socket
            .send(Message::Text(
                json!({"type":"input_audio_buffer.committed","item_id":"item-1"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        socket.send(Message::Text(json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"item-1","transcript":"尾音完整🙂"}).to_string().into())).await.unwrap();
    });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{addr}"))
        .await
        .unwrap();
    let (input, mut events, worker) = actor(false, socket).await;
    input.append_pcm16_16k(&pcm()).unwrap();
    assert_eq!(
        events.recv().await.unwrap(),
        StreamingAsrEvent::Partial {
            text: "尾音".into()
        }
    );
    input.finish().await.unwrap();
    assert_eq!(worker.await.unwrap().unwrap(), "尾音完整🙂");
    peer.await.unwrap();
}

#[tokio::test]
async fn cancellation_bypasses_a_full_audio_queue() {
    let (tx, _rx) = mpsc::channel(1);
    let input = StreamingAsrInput::new(tx, CancellationToken::new());
    assert!(input.is_accepting());
    input.append_pcm16_16k(&pcm()).unwrap();
    assert!(input.append_pcm16_16k(&pcm()).is_err());
    assert!(input.cancel.is_cancelled());
    assert!(input.finish().await.is_err());
}

#[tokio::test]
async fn finish_is_lossless_and_cancellation_safe_with_a_full_audio_queue() {
    let (tx, mut rx) = mpsc::channel(AUDIO_QUEUE);
    let input = StreamingAsrInput::new(tx, CancellationToken::new());
    for _ in 0..AUDIO_QUEUE {
        input.append_pcm16_16k(&pcm()).unwrap();
    }
    // Previously this timed out after setting finishing=true, permanently
    // losing the queued EOS. A second call then misleadingly returned Ok.
    tokio::time::timeout(Duration::from_millis(10), input.finish())
        .await
        .expect("EOS must not compete for an audio slot")
        .unwrap();
    input.finish().await.unwrap();
    assert!(input.finish_requested.is_cancelled());
    assert!(!input.is_accepting());
    assert!(input.append_pcm16_16k(&pcm()).is_err());
    rx.close();
    let mut drained = 0;
    while let Some(Command::Pcm16 { samples, .. }) = rx.recv().await {
        drained += samples.len();
    }
    assert_eq!(drained, AUDIO_QUEUE * 960);
}

#[test]
fn internal_input_failure_is_an_error_and_never_a_user_cancellation() {
    let (tx, _rx) = mpsc::channel(1);
    let input = StreamingAsrInput::new(tx, CancellationToken::new());
    input.append_pcm16_16k(&pcm()).unwrap();
    assert!(input.append_pcm16_16k(&pcm()).is_err());
    assert!(matches!(
        terminal_event(Err("ASR cancelled".into()), &input.state, &input.cancel),
        StreamingAsrEvent::Error { .. }
    ));
    assert!(input.state.lock().unwrap().closed);
    assert!(!input.is_accepting());
    let (tx, _rx) = mpsc::channel(1);
    let explicit = StreamingAsrInput::new(tx, CancellationToken::new());
    explicit.cancel();
    assert_eq!(
        terminal_event(
            Err("ASR cancelled".into()),
            &explicit.state,
            &explicit.cancel
        ),
        StreamingAsrEvent::Cancelled
    );
    assert_eq!(
        terminal_event(Ok("迟到结果".into()), &explicit.state, &explicit.cancel),
        StreamingAsrEvent::Cancelled
    );
}

#[tokio::test]
async fn stepfun_repeated_sessions_drain_every_pcm_before_one_commit_and_match_created_sources() {
    const ROUNDS: usize = 32;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        for round in 0..ROUNDS {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            socket
                .send(Message::Text(
                    json!({"type":"session.created"}).to_string().into(),
                ))
                .await
                .unwrap();
            let update: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            assert_eq!(update["type"], "session.update");
            assert_eq!(update["session"]["audio"]["input"]["format"]["rate"], 16000);
            assert_eq!(
                update["session"]["audio"]["input"]["turn_detection"],
                json!({})
            );
            assert_eq!(
                update["session"]["audio"]["input"]["transcription"]["full_rerun_on_commit"],
                json!(round % 2 != 0),
                "both second-pass modes require the same unique final receipts"
            );
            socket
                .send(Message::Text(
                    json!({"type":"session.updated","session":update["session"]})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
            let buffer = format!("buffer-{round}");
            let source = format!("source-{round}");
            let mut bytes = 0;
            loop {
                let message = socket.next().await.unwrap().unwrap();
                if let Message::Ping(data) = message {
                    socket.send(Message::Pong(data)).await.unwrap();
                    continue;
                }
                let raw: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
                match raw["type"].as_str().unwrap() {
                    "input_audio_buffer.append" => {
                        if bytes == 0 {
                            for (id, status) in [(&buffer, "completed"), (&source, "in_progress")] {
                                socket.send(Message::Text(json!({"type":"conversation.item.created","item":{"id":id,"status":status,"role":"user","content":[{"type":"input_audio"}]}}).to_string().into())).await.unwrap();
                            }
                            socket.send(Message::Text(json!({"type":"conversation.item.input_audio_transcription.delta","item_id":source,"text":"","stash":"候选"}).to_string().into())).await.unwrap();
                        }
                        bytes += base64::engine::general_purpose::STANDARD
                            .decode(raw["audio"].as_str().unwrap())
                            .unwrap()
                            .len();
                    }
                    "input_audio_buffer.commit" => break,
                    other => panic!("unexpected client event {other}"),
                }
            }
            let samples_per_chunk = [1, 640, 960, 959][round % 4];
            assert_eq!(bytes, samples_per_chunk * 2 * AUDIO_QUEUE);
            let final_ = json!({"type":"conversation.item.input_audio_transcription.completed","item_id":source,"transcript":"本段。"});
            let commit = json!({"type":"input_audio_buffer.committed","item_id":buffer});
            // Both provider response orders must require the same two facts.
            for raw in if round % 2 == 0 {
                [commit, final_]
            } else {
                [final_, commit]
            } {
                socket
                    .send(Message::Text(raw.to_string().into()))
                    .await
                    .unwrap();
            }
            let next = tokio::time::timeout(Duration::from_secs(1), socket.next())
                .await
                .unwrap();
            assert!(
                !matches!(next, Some(Ok(Message::Text(_)))),
                "no second commit or audio after EOS"
            );
        }
    });
    for round in 0..ROUNDS {
        let mut call = local_stepfun_call(address);
        call.provider_params = json!({"full_rerun_on_commit":round % 2 != 0});
        let (socket, wire) = prepare_session(
            async { Ok(call) },
            &StreamingAsrConfig::default(),
        )
        .await
        .unwrap();
        let (tx, commands) = mpsc::channel(AUDIO_QUEUE);
        let (events, _rx) = mpsc::channel(64);
        let cancel = CancellationToken::new();
        let input = StreamingAsrInput::new(tx, cancel.clone());
        let requested = input.finish_requested.clone();
        let worker =
            tokio::spawn(
                async move { run(socket, wire, commands, &events, cancel, requested).await },
            );
        let samples_per_chunk = [1, 640, 960, 959][round % 4];
        for _ in 0..AUDIO_QUEUE {
            input.append_pcm16_16k(&vec![8; samples_per_chunk]).unwrap();
        }
        input.finish().await.unwrap();
        input.finish().await.unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(2), worker)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            "本段。"
        );
        assert!(input.append_pcm16_16k(&pcm()).is_err());
    }
    peer.await.unwrap();
}

#[tokio::test]
async fn stepfun_configuration_does_not_force_second_pass_and_preserves_explicit_values() {
    for rerun in [None, Some(false), Some(true)] {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let peer = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let update: Value =
                serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                    .unwrap();
            let transcription = &update["session"]["audio"]["input"]["transcription"];
            assert_eq!(
                transcription.get("full_rerun_on_commit"),
                rerun.map(Value::Bool).as_ref(),
                "an absent catalog setting uses the provider default"
            );
            socket.send(Message::Text(
                json!({"type":"session.updated","session":update["session"]})
                    .to_string().into()
            )).await.unwrap();
        });
        let mut call = local_stepfun_call(address);
        if let Some(rerun) = rerun {
            call.provider_params = json!({"full_rerun_on_commit":rerun});
        }
        let (socket, _) = prepare_session(async { Ok(call) }, &StreamingAsrConfig::default())
            .await.unwrap();
        drop(socket);
        peer.await.unwrap();
    }
}

#[tokio::test]
async fn provider_event_flood_cannot_starve_audio_drain_and_eos() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let socket = accept_async(stream).await.unwrap();
        let (mut sink, mut stream) = socket.split();
        let flood = tokio::spawn(async move {
            let message = Message::Text(
                json!({"type":"session.updated","ignored":"x".repeat(8192)})
                    .to_string()
                    .into(),
            );
            loop {
                if sink.send(message.clone()).await.is_err() {
                    break;
                }
            }
        });
        let mut bytes = 0;
        while let Some(Ok(message)) = stream.next().await {
            if !message.is_text() {
                continue;
            }
            let raw: Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
            match raw["type"].as_str().unwrap() {
                "input_audio_buffer.append" => {
                    bytes += base64::engine::general_purpose::STANDARD
                        .decode(raw["audio"].as_str().unwrap())
                        .unwrap()
                        .len()
                }
                "input_audio_buffer.commit" => break,
                _ => {}
            }
        }
        assert_eq!(bytes, 2880 * AUDIO_QUEUE); // OpenAI PCM is resampled to 24 kHz.
        flood.abort();
        let _ = flood.await;
    });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
        .await
        .unwrap();
    let (input, _events, worker) = actor(false, socket).await;
    for _ in 0..AUDIO_QUEUE {
        input.append_pcm16_16k(&pcm()).unwrap();
    }
    input.finish().await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), peer)
        .await
        .expect("provider reads cannot monopolize audio/EOS admission")
        .unwrap();
    assert!(
        worker.await.unwrap().is_err(),
        "provider EOF is not a final result"
    );
}

#[tokio::test]
async fn stepfun_stream_replaces_cumulative_partial_and_uses_pcm_without_resampling() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let append: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert!(append["event_id"].is_string());
        assert_eq!(append["type"], "input_audio_buffer.append");
        assert_eq!(
            base64::engine::general_purpose::STANDARD
                .decode(append["audio"].as_str().unwrap())
                .unwrap()
                .len(),
            1920
        );
        for id in ["buffer-1", "item-1"] {
            socket.send(Message::Text(json!({"type":"conversation.item.created","item":{"id":id,"type":"message","role":"user","content":[{"type":"input_audio"}]}}).to_string().into())).await.unwrap();
        }
        for (text, stash) in [("明天三点", ""), ("可以", "可以"), ("明天下午三点", "")]
        {
            socket.send(Message::Text(json!({"type":"conversation.item.input_audio_transcription.delta","item_id":"item-1","text":text,"stash":stash}).to_string().into())).await.unwrap();
        }
        let commit: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(commit["type"], "input_audio_buffer.commit");
        assert!(commit["event_id"].is_string());
        socket
            .send(Message::Text(
                json!({"type":"input_audio_buffer.committed","item_id":"buffer-1"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        socket.send(Message::Text(json!({"type":"conversation.item.input_audio_transcription.completed","item_id":"item-1","transcript":"明天下午三点。"}).to_string().into())).await.unwrap();
    });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
        .await
        .unwrap();
    let (tx, commands) = mpsc::channel(AUDIO_QUEUE);
    let (events, mut event_rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    let input = StreamingAsrInput::new(tx, cancel.clone());
    let finish_requested = input.finish_requested.clone();
    let worker = tokio::spawn(async move {
        run(
            socket,
            Wire::StepFun,
            commands,
            &events,
            cancel,
            finish_requested,
        )
        .await
    });
    input.append_pcm16_16k(&pcm()).unwrap();
    assert_eq!(
        event_rx.recv().await.unwrap(),
        StreamingAsrEvent::Partial {
            text: "明天三点".into()
        }
    );
    assert_eq!(
        event_rx.recv().await.unwrap(),
        StreamingAsrEvent::Partial {
            text: "可以可以".into()
        }
    );
    assert_eq!(
        event_rx.recv().await.unwrap(),
        StreamingAsrEvent::Partial {
            text: "明天下午三点".into()
        }
    );
    input.finish().await.unwrap();
    assert_eq!(worker.await.unwrap().unwrap(), "明天下午三点。");
    server.await.unwrap();
}

#[test]
fn stepfun_buffer_receipt_never_finalizes_provisional_or_multiple_sources() {
    let mut transcript = Transcript::default();
    let mut items = StepFunItems::default();
    items.created(&json!({"item":{"id":"buffer-1"}})).unwrap();
    items.created(&json!({"item":{"id":"source-1"}})).unwrap();
    items.source("source-1").unwrap();
    transcript
        .put("source-1".into(), "候选".into(), false)
        .unwrap();
    assert!(!transcript.final_ready(items.committed_source(Some("buffer-1"))));
    transcript
        .put("source-1".into(), "正文".into(), true)
        .unwrap();
    assert!(!transcript.final_ready(items.committed_source(None)));
    assert!(!transcript.final_ready(Some("buffer-1")));
    assert!(!transcript.final_ready(items.committed_source(Some("unknown-buffer"))));
    assert!(transcript.final_ready(items.committed_source(Some("buffer-1"))));
    assert!(items.source("unannounced-source").is_err());
    items.created(&json!({"item":{"id":"source-2"}})).unwrap();
    assert!(items.source("source-2").is_err());
}

#[tokio::test]
async fn digital_zero_pcm_is_a_signal_error_and_never_an_inferred_empty_final() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let append: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(append["type"], "input_audio_buffer.append");
        let remaining = tokio::time::timeout(Duration::from_secs(1), socket.next())
            .await
            .unwrap();
        if let Some(Ok(Message::Text(text))) = remaining {
            let value: Value = serde_json::from_str(&text).unwrap();
            assert_ne!(
                value["type"], "input_audio_buffer.commit",
                "zero input must fail instead of claiming provider completion"
            );
        }
    });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
        .await
        .unwrap();
    let (tx, commands) = mpsc::channel(AUDIO_QUEUE);
    let (events, _rx) = mpsc::channel(64);
    let cancel = CancellationToken::new();
    let input = StreamingAsrInput::new(tx, cancel.clone());
    let finish_requested = input.finish_requested.clone();
    let worker = tokio::spawn(async move {
        run(
            socket,
            Wire::StepFun,
            commands,
            &events,
            cancel,
            finish_requested,
        )
        .await
    });
    input.append_pcm16_16k(&[0; 960]).unwrap();
    input.finish().await.unwrap();
    assert!(
        worker
            .await
            .unwrap()
            .unwrap_err()
            .contains("未检测到麦克风输入信号")
    );
    peer.await.unwrap();
}

fn local_stepfun_call(address: std::net::SocketAddr) -> ResolvedTaskConfig {
    ResolvedTaskConfig {
        provider_id: "local-lifecycle-test".into(),
        config_revision: 1,
        platform: "stepfun".into(),
        model: "stepaudio-2.5-asr-stream".into(),
        task: ModelTask::SpeechRecognition,
        traits: vec![], input_constraints: None,
        protocol: "stepfun.asr_stream".into(),
        connection: crate::ResolvedConnection {
            role: "default".into(),
            base_url: format!("http://{address}"),
            auth: crate::AuthMaterial {
                scheme: AuthScheme::Bearer,
                credentials: json!({"api_keys":["local-fixture-only"]}),
            },
            extra: json!({}),
        },
        transport: crate::ResolvedTaskTransport {
            realtime_endpoint: Some(format!("ws://{address}/stream")),
            ..Default::default()
        },
        provider_params: json!({}),
        context_limit: None,
        output_limit: None,
        bedrock_config: None,
    }
}

#[tokio::test]
async fn preparation_deadline_includes_catalog_resolution() {
    struct ResolutionLease(Arc<std::sync::atomic::AtomicBool>);
    impl Drop for ResolutionLease {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let resolver = async {
        let _lease = ResolutionLease(dropped.clone());
        std::future::pending::<Result<ResolvedTaskConfig, InvokeError>>().await
    };
    let error = prepare_session(resolver, &StreamingAsrConfig::default())
        .await
        .err()
        .expect("an unresolved catalog cannot remain readying indefinitely");
    assert_eq!(error.kind, InvokeErrorKind::Timeout);
    assert_eq!(
        error.message,
        "streaming ASR preparation timed out after 20 seconds"
    );
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn preparation_deadline_is_shared_by_resolution_and_provider_configuration() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let peer = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        let update: Value =
            serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap()).unwrap();
        assert_eq!(update["type"], "session.update");
        // No configuration receipt. The overall deadline must drop this
        // socket, rather than grant a fresh 15 seconds after resolution.
        let next = socket.next().await;
        assert!(!matches!(next, Some(Ok(Message::Text(_)))));
    });
    let resolver = async {
        tokio::time::sleep(Duration::from_secs(7)).await;
        Ok(local_stepfun_call(address))
    };
    let error = prepare_session(resolver, &StreamingAsrConfig::default())
        .await
        .err()
        .expect("configuration must fit inside the same readiness deadline");
    assert_eq!(error.kind, InvokeErrorKind::Timeout);
    assert_eq!(
        error.message,
        "streaming ASR preparation timed out after 20 seconds"
    );
    tokio::time::timeout(Duration::from_secs(1), peer)
        .await
        .expect("timed-out preparation must release its socket")
        .unwrap();
}

#[tokio::test]
async fn idle_provider_close_or_eof_fails_without_finishing_or_accepting_late_audio() {
    for deepgram in [false, true] {
        for graceful in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let peer = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = accept_async(stream).await.unwrap();
                if graceful {
                    socket.close(None).await.unwrap();
                }
                // Dropping without a close frame exercises transport EOF.
            });
            let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
                .await
                .unwrap();
            let (input, mut events, worker) = actor(deepgram, socket).await;
            let message = tokio::time::timeout(Duration::from_secs(1), worker)
                .await
                .expect("idle provider closure must be noticed before recording")
                .unwrap()
                .unwrap_err();
            if graceful {
                assert_eq!(message, "ASR closed before its final acknowledgement");
            } else {
                assert!(matches!(
                    message.as_str(),
                    "ASR transport failed" | "ASR disconnected before finalization"
                ));
            }
            assert!(events.recv().await.is_none());
            assert!(input.append_pcm16_16k(&pcm()).is_err());
            assert!(input.finish().await.is_err());
            peer.await.unwrap();
        }
    }
}
