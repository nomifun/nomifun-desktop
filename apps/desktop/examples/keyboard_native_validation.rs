//! Production subtitle Actor with explicit synthetic PCM and loopback models.
//! This example never opens a microphone, serial port, user profile or clipboard.
#[path = "../src/keyboard/mod.rs"]
mod keyboard;
use anyhow::{Context, Result, ensure};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{State, WebSocketUpgrade, ws::{Message, WebSocket}},
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    response::IntoResponse,
    routing::{get, post},
};
use futures_util::StreamExt;
use nomifun_db::{
    CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository,
};
use nomifun_keyboard::document::{AsrMode, ModelSelection, Settings, TextSelection};
use nomifun_model_invoke::{AdapterRegistry, ModelInvokeService, default_adapters};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::{AtomicBool, AtomicUsize, Ordering}},
    time::Duration,
};

#[derive(Clone, Default)]
struct Fixture {
    platform_asr: bool,
    pcm: Arc<AtomicUsize>,
    uploads: Arc<AtomicUsize>,
    connections: Arc<AtomicUsize>,
    hold_handshakes: Arc<AtomicBool>,
    hold_final: Arc<AtomicBool>,
    final_requests: Arc<AtomicUsize>,
    final_attempts: Arc<AtomicUsize>,
    asr_text: Arc<Mutex<String>>,
    polish: Arc<AtomicUsize>,
    hold_polish_final: Arc<AtomicBool>,
    polish_attempts: Arc<AtomicUsize>,
    upload_before_stop: Arc<AtomicBool>,
    app: Arc<Mutex<Option<tauri::AppHandle>>>,
}
fn count(counter: &AtomicUsize) -> usize { counter.load(Ordering::Acquire) }
async fn gate(flag: &AtomicBool) {
    while flag.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
async fn socket(mut socket: WebSocket, fixture: Fixture) {
    let mut partial = false;
    while let Some(Ok(message)) = socket.next().await {
        match message {
            Message::Binary(bytes) => {
                fixture.pcm.fetch_add(bytes.len() / 2, Ordering::Release);
                if !partial {
                    partial = true;
                    let _ = socket.send(Message::Text(json!({"type":"Results","start":0,
                        "is_final":false,"channel":{"alternatives":[{"transcript":"临时字幕"}]}}
                    ).to_string().into())).await;
                }
            }
            Message::Text(raw) if serde_json::from_str::<Value>(&raw).ok()
                .is_some_and(|value| value["type"] == "CloseStream") => {
                let text = fixture.asr_text.lock().unwrap().clone();
                fixture.final_requests.fetch_add(1, Ordering::Release);
                // Keep work alive after the client cancels. The test proves
                // a provider's late completion cannot revive an operation.
                gate(&fixture.hold_final).await;
                fixture.final_attempts.fetch_add(1, Ordering::Release);
                let _ = socket.send(Message::Text(json!({"type":"Results","start":0,
                    "is_final":true,"channel":{"alternatives":[{"transcript":text}]}}
                ).to_string().into())).await;
                let _ = socket.send(Message::Text(json!({"type":"Metadata","duration":1})
                    .to_string().into())).await;
                break;
            }
            Message::Ping(value) => { let _ = socket.send(Message::Pong(value)).await; }
            Message::Close(_) => break,
            _ => {}
        }
    }
}
async fn chat(State(fixture): State<Fixture>, Json(body): Json<Value>) -> axum::response::Response {
    let messages = body["messages"].as_array();
    let content = &body["messages"][1]["content"];
    let text = content.as_str().map(str::to_owned).or_else(|| content.as_array().map(|parts|
        parts.iter().filter_map(|part| part["text"].as_str()).collect::<String>()));
    if body.get("tools").is_some() || !messages.is_some_and(|messages| messages.len() == 2)
        || body["messages"][0]["role"] != "system" || body["messages"][1]["role"] != "user"
        || text.as_ref().is_none_or(|text| text.is_empty()) {
        return (StatusCode::BAD_REQUEST, "unexpected model body or tools").into_response();
    }
    fixture.polish.fetch_add(1, Ordering::Release);
    let candidate = format!("美化：{}", text.unwrap());
    let first = json!({"id":"fixture","object":"chat.completion.chunk","choices":[{
        "index":0,"delta":{"role":"assistant","content":candidate},"finish_reason":null}]}).to_string();
    let last = json!({"id":"fixture","object":"chat.completion.chunk","choices":[{
        "index":0,"delta":{},"finish_reason":"stop"}]}).to_string();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Bytes, std::convert::Infallible>>(2);
    tokio::spawn(async move {
        let _ = tx.send(Ok(Bytes::from(format!("data: {first}\n\n")))).await;
        gate(&fixture.hold_polish_final).await;
        fixture.polish_attempts.fetch_add(1, Ordering::Release);
        let _ = tx.send(Ok(Bytes::from(format!("data: {last}\n\ndata: [DONE]\n\n")))).await;
    });
    let stream = futures_util::stream::unfold(rx, |mut rx| async {
        rx.recv().await.map(|chunk| (chunk, rx))
    });
    ([("content-type", "text/event-stream")], axum::body::Body::from_stream(stream)).into_response()
}

fn find_bytes(body: &[u8], needle: &[u8]) -> Option<usize> {
    body.windows(needle.len()).position(|part| part == needle)
}

/// Validate the actual uploaded file, not an arbitrary RIFF sequence elsewhere
/// in the multipart body. Audio stays in memory inside the loopback fixture.
fn uploaded_pcm_samples(headers: &HeaderMap, body: &[u8]) -> Result<usize> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| anyhow::anyhow!("missing multipart content type"))?;
    ensure!(
        content_type
            .split(';')
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("multipart/form-data")),
        "expected multipart audio upload"
    );
    let boundary = content_type
        .split(';')
        .skip(1)
        .find_map(|value| value.trim().strip_prefix("boundary="))
        .map(|value| value.trim_matches('"'))
        .ok_or_else(|| anyhow::anyhow!("multipart boundary missing"))?;
    ensure!(
        !boundary.is_empty() && boundary.len() <= 200 && !boundary.contains(['\r', '\n']),
        "invalid multipart boundary"
    );
    let marker = format!("--{boundary}").into_bytes();
    let next = format!("\r\n--{boundary}").into_bytes();
    let mut cursor = 0;
    let mut audio = None;
    let mut model = None;
    loop {
        ensure!(
            body.get(cursor..cursor + marker.len()) == Some(marker.as_slice()),
            "invalid multipart delimiter"
        );
        cursor += marker.len();
        if body.get(cursor..cursor + 2) == Some(b"--") {
            break;
        }
        ensure!(
            body.get(cursor..cursor + 2) == Some(b"\r\n"),
            "invalid multipart part start"
        );
        cursor += 2;
        let header_end = find_bytes(&body[cursor..], b"\r\n\r\n")
            .ok_or_else(|| anyhow::anyhow!("multipart part headers missing"))?
            + cursor;
        let part_headers = std::str::from_utf8(&body[cursor..header_end])?;
        let data_start = header_end + 4;
        let data_end = find_bytes(&body[data_start..], &next)
            .ok_or_else(|| anyhow::anyhow!("multipart part end missing"))?
            + data_start;
        let disposition = part_headers
            .lines()
            .find(|line| {
                line.to_ascii_lowercase()
                    .starts_with("content-disposition:")
            })
            .ok_or_else(|| anyhow::anyhow!("multipart content disposition missing"))?;
        let field = disposition
            .split(';')
            .find_map(|value| value.trim().strip_prefix("name=\""))
            .and_then(|value| value.strip_suffix('"'));
        match field {
            Some("file") => {
                ensure!(audio.is_none(), "duplicate audio file");
                audio = Some(&body[data_start..data_end]);
            }
            Some("model") => {
                ensure!(model.is_none(), "duplicate model field");
                model = Some(&body[data_start..data_end]);
            }
            _ => {}
        }
        cursor = data_end + 2;
    }
    ensure!(
        model == Some(b"file-model".as_slice()),
        "unexpected transcription model"
    );
    let wave = audio.ok_or_else(|| anyhow::anyhow!("multipart file missing"))?;
    ensure!(
        wave.len() >= 44 && &wave[..4] == b"RIFF" && &wave[8..12] == b"WAVE",
        "uploaded audio is not RIFF/WAVE"
    );
    let u32_at =
        |offset: usize| u32::from_le_bytes(wave[offset..offset + 4].try_into().unwrap()) as usize;
    ensure!(
        u32_at(4).checked_add(8) == Some(wave.len()),
        "WAVE RIFF length mismatch"
    );
    let mut cursor = 12;
    let mut format = false;
    let mut samples = None;
    while cursor < wave.len() {
        ensure!(cursor + 8 <= wave.len(), "truncated WAVE chunk header");
        let length = u32_at(cursor + 4);
        let start = cursor + 8;
        let end = start
            .checked_add(length)
            .ok_or_else(|| anyhow::anyhow!("WAVE chunk overflow"))?;
        ensure!(end <= wave.len(), "truncated WAVE chunk");
        match &wave[cursor..cursor + 4] {
            b"fmt " => {
                ensure!(!format && length >= 16, "invalid WAVE format chunk");
                let u16_at = |offset: usize| {
                    u16::from_le_bytes(wave[offset..offset + 2].try_into().unwrap())
                };
                ensure!(
                    u16_at(start) == 1
                        && u16_at(start + 2) == 1
                        && u32_at(start + 4) == 16_000
                        && u32_at(start + 8) == 32_000
                        && u16_at(start + 12) == 2
                        && u16_at(start + 14) == 16,
                    "expected 16 kHz mono PCM16 WAVE"
                );
                format = true;
            }
            b"data" => {
                ensure!(
                    samples.is_none() && length > 0 && length % 2 == 0,
                    "invalid WAVE PCM data"
                );
                samples = Some(length / 2);
            }
            _ => {}
        }
        cursor = end + length % 2;
    }
    ensure!(format && cursor == wave.len(), "incomplete WAVE file");
    samples.ok_or_else(|| anyhow::anyhow!("WAVE PCM data chunk missing"))
}

async fn transcribe(State(fixture): State<Fixture>, headers: HeaderMap, body: Bytes) -> impl IntoResponse {
    fixture.uploads.fetch_add(1, Ordering::Release);
    let samples = match uploaded_pcm_samples(&headers, &body) {
        Ok(samples) => samples,
        Err(error) => return (StatusCode::BAD_REQUEST, Json(json!({"error":{"message":error.to_string()}}))),
    };
    let stopped = fixture.app.lock().unwrap().as_ref().is_some_and(|app| {
        let state = keyboard::validation_snapshot(app);
        state["audioState"]["state"] == "stopped" && state["phase"] == "finalizing"
    });
    if !stopped { fixture.upload_before_stop.store(true, Ordering::Release); }
    fixture.pcm.fetch_add(samples, Ordering::Release);
    let text = fixture.asr_text.lock().unwrap().clone();
    fixture.final_requests.fetch_add(1, Ordering::Release);
    let late = fixture.clone();
    tokio::spawn(async move {
        gate(&late.hold_final).await;
        late.final_attempts.fetch_add(1, Ordering::Release);
    }).await.expect("loopback ASR work");
    (StatusCode::OK, Json(json!({"text":text})))
}
async fn fixtures(platform_asr: bool, database_path: &std::path::Path)
    -> Result<(Arc<ModelInvokeService>, Settings, Fixture)> {
    let fixture = Fixture { platform_asr, ..Default::default() };
    let router = Router::new()
        .route("/v1/listen", get(|ws: WebSocketUpgrade, State(f): State<Fixture>| async move {
            f.connections.fetch_add(1, Ordering::Release);
            gate(&f.hold_handshakes).await;
            ws.on_upgrade(move |socket_stream| socket(socket_stream, f)).into_response()
        }))
        .route("/v1/chat/completions", post(chat))
        .route("/v1/audio/transcriptions", post(transcribe))
        .with_state(fixture.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    tokio::spawn(async move { let _ = axum::serve(listener, router).await; });
    // A cancelled SQLite acquisition may retire its connection. A private
    // file keeps the fixture schema alive across that legitimate cancellation.
    let database = nomifun_db::init_database(database_path).await?;
    let pool = database.pool().clone();
    let key = [0x39; 32];
    let encrypted = nomifun_common::encrypt_string(r#"{"api_keys":["local-fixture-only"]}"#, &key)?;
    let repo = SqliteProviderRepository::new(pool.clone());
    let mut refs = Vec::new();
    for (model, task, protocol, endpoint, realtime) in [
        (
            "fixture-transcription",
            "speech_recognition",
            "deepgram.listen_stream",
            None,
            Some("/v1/listen"),
        ),
        (
            "fixture-editor",
            "chat",
            "openai.chat_text",
            Some("/v1/chat/completions"),
            None,
        ),
        (
            "file-model",
            "speech_recognition",
            "openai.audio_transcriptions",
            Some("/v1/audio/transcriptions"),
            None,
        ),
    ] {
        let capabilities = [NewProviderModelCapability {
            task,
            traits: "[]",
            protocol,
            connection_role: "default",
            endpoint,
            realtime_endpoint: realtime,
            provider_params: "{}",
            ..Default::default()
        }];
        let (provider, _) = repo
            .create(
                CreateProviderParams {
                    provider_id: None,
                    platform: "custom",
                    name: "Isolated native keyboard fixture",
                    base_url: &base,
                    auth_scheme: "bearer",
                    credentials_encrypted: &encrypted,
                    enabled: true,
                    bedrock_config: None,
                    sort_order: None,
                },
                &NewProviderModel {
                    model,
                    enabled: true,
                    capabilities: &capabilities,
                    ..Default::default()
                },
                &[],
            )
            .await?;
        refs.push(ModelSelection {
            provider_id: provider.provider_id,
            model: model.into(),
        });
    }
    let service = Arc::new(ModelInvokeService::new(
        Arc::new(repo),
        Arc::new(SqliteProviderModelRepository::new(pool.clone())),
        Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
        Arc::new(SqliteProviderConnectionRepository::new(pool)),
        key,
        reqwest::Client::builder().no_proxy().build()?,
        AdapterRegistry::new(default_adapters()),
    ));
    let settings = Settings {
        asr_mode: if platform_asr {
            AsrMode::Platform
        } else {
            AsrMode::Streaming
        },
        asr_model: Some(refs[if platform_asr { 2 } else { 0 }].clone()),
        text_model: Some(refs[1].clone()),
        ..Default::default()
    };
    Ok((service, settings, fixture))
}

async fn wait_state(app: &tauri::AppHandle, predicate: impl Fn(&Value) -> bool) -> Result<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = keyboard::validation_snapshot(app);
        if predicate(&state) { return Ok(state); }
        ensure!(tokio::time::Instant::now() < deadline,
            "state timeout: phase={} audio={} preparing={} error={}",
            state["phase"], state["audioState"]["state"], state["capturePreparing"], state["error"]);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn wait_counter(counter: &AtomicUsize, expected: usize) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while count(counter) < expected {
        ensure!(tokio::time::Instant::now() < deadline, "provider counter timeout: {} < {expected}", count(counter));
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}
async fn action(app: &tauri::AppHandle, name: &str) -> Result<Value> {
    keyboard::validation_request(app, Some(name), None).await.map_err(anyhow::Error::msg)
        .with_context(|| format!("UI action {name}"))
}
async fn hardware(app: &tauri::AppHandle, pressed: Option<bool>) -> Result<Value> {
    keyboard::validation_hardware(app, pressed).await.map_err(anyhow::Error::msg)
}
async fn click(app: &tauri::AppHandle, key: &str) -> Result<Value> {
    keyboard::validation_hardware_click(app, key).await.map_err(anyhow::Error::msg)
}
fn document(state: &Value) -> Option<String> { state["documentId"].as_str().map(str::to_owned) }
async fn captured_action(app: &tauri::AppHandle, name: &str, state: &Value) -> Result<Value> {
    keyboard::validation_document_operation(app, name, document(state), revision(state)?, state["cancelGeneration"].as_u64())
        .await.map_err(anyhow::Error::msg).with_context(|| format!("captured {name}: old_doc={} current_doc={} old_revision={} current_revision={} old_cancel={} current_cancel={}",
            state["documentId"], keyboard::validation_snapshot(app)["documentId"], state["revision"],
            keyboard::validation_snapshot(app)["revision"], state["cancelGeneration"], keyboard::validation_snapshot(app)["cancelGeneration"]))
}
fn revision(state: &Value) -> Result<u64> { state["revision"].as_u64().context("revision missing") }
async fn edit(app: &tauri::AppHandle, text: &str, lease: Option<&str>) -> Result<Value> {
    let state = keyboard::validation_snapshot(app);
    keyboard::validation_edit(app, text.into(), document(&state), revision(&state)?, lease.map(str::to_owned))
        .await.map_err(anyhow::Error::msg)
}
async fn editor(app: &tauri::AppHandle, action: &str, id: &str, selection: Option<TextSelection>) -> Result<Value> {
    let state = keyboard::validation_snapshot(app);
    keyboard::validation_edit_selection(app, action, id, document(&state), revision(&state)?, selection)
        .await.map_err(anyhow::Error::msg)
}
fn idle(state: &Value) -> bool {
    state["phase"] == "idle" && state["audioState"]["state"] == "stopped" && state["capturePreparing"] == false
}
async fn settled(app: &tauri::AppHandle, text: &str) -> Result<Value> {
    let state = wait_state(app, |state| idle(state) && state["text"] == text).await?;
    ensure!(state["error"].is_null() && state["partial"] == "" && state["candidate"] == ""
        && state["pendingAction"].is_null() && state["operationId"].is_null(),
        "settled draft has transient state: {}", state["error"]);
    Ok(state)
}
async fn start(app: &tauri::AppHandle, fixture: &Fixture, text: &str) -> Result<Value> {
    *fixture.asr_text.lock().unwrap() = text.into();
    hardware(app, Some(true)).await?;
    let state = wait_state(app, |state| state["phase"] == "recording"
        && state["audioState"]["state"] == "capturing").await?;
    // Two fixed 60 ms blocks exercise a real nonempty native CaptureInput.
    tokio::time::sleep(Duration::from_millis(75)).await;
    Ok(state)
}
async fn assert_stable(app: &tauri::AppHandle, expected: &str) -> Result<Value> {
    tokio::time::sleep(Duration::from_millis(100)).await;
    settled(app, expected).await
}
async fn close(app: &tauri::AppHandle) -> Result<Value> {
    action(app, "close").await?;
    let state = wait_state(app, |state| idle(state) && state["visible"] == false).await?;
    ensure!(state["text"] == "" && state["originalText"] == "" && state["partial"] == ""
        && state["candidate"] == "" && state["documentId"].is_null()
        && state["editSessionId"].is_null() && state["dictationSelection"].is_null(), "close did not discard draft");
    Ok(state)
}
async fn preparation_and_editor(app: &tauri::AppHandle, fixture: &Fixture) -> Result<Value> {
    action(app, "open").await?;
    edit(app, "编辑前正文", None).await?;
    for (key, id, draft) in [("k4", "cancel-capture-edit", "未丢失的编辑🙂"),
        ("k2", "cancel-polish-edit", "新的未丢失编辑🙂")] {
        editor(app, "edit_begin", id, None).await?;
        let finals = count(&fixture.final_requests);
        let polish = count(&fixture.polish);
        if key == "k4" { hardware(app, Some(true)).await?; } else { click(app, key).await?; }
        let pending = keyboard::validation_snapshot(app);
        ensure!(pending["editSessionId"] == id && pending["phase"] == "idle", "editor lease was not retained while awaiting flush");
        if key == "k4" { ensure!(pending["capturePreparing"] == true, "editor capture not pending"); }
        let old_cancel_generation = pending["cancelGeneration"].as_u64().context("cancel generation missing")?;
        click(app, "k3").await?;
        hardware(app, Some(false)).await?;
        let cancelled = wait_state(app, idle).await?;
        ensure!(cancelled["editSessionId"] == id, "K3 discarded unsaved editor lease");
        edit(app, draft, Some(id)).await?;
        editor(app, "edit_end", id, Some(TextSelection { anchor: draft.encode_utf16().count() as u64,
            focus: draft.encode_utf16().count() as u64 })).await?;
        assert_stable(app, draft).await?;
        for deferred in ["beautify", "restore_original"] {
            let mut late_request = keyboard::validation_snapshot(app);
            late_request["cancelGeneration"] = json!(old_cancel_generation);
            ensure!(captured_action(app, deferred, &late_request).await.is_err(),
                "cancelled UI {deferred} resumed after a late editor save");
        }
        assert_stable(app, draft).await?;
        ensure!(count(&fixture.final_requests) == finals && count(&fixture.polish) == polish,
            "cancelled editor flush replayed a capture or model operation");
    }
    // An already queued editor save can reach Rust before the user's cancel.
    // The save's new body revision must not revoke that document-level cancel.
    for (key, id, draft) in [("k4", "save-before-cancel-capture", "先保存后取消的正文"),
        ("k2", "save-before-cancel-polish", "第二份先保存后取消的正文")] {
        editor(app, "edit_begin", id, None).await?;
        let finals = count(&fixture.final_requests);
        let polish = count(&fixture.polish);
        if key == "k4" { hardware(app, Some(true)).await?; } else { click(app, key).await?; }
        let request = keyboard::validation_snapshot(app);
        edit(app, draft, Some(id)).await?;
        ensure!(keyboard::validation_snapshot(app)["revision"] != request["revision"], "save did not advance revision");
        captured_action(app, "cancel", &request).await?;
        captured_action(app, "cancel", &request).await?;
        hardware(app, Some(false)).await?;
        ensure!(keyboard::validation_snapshot(app)["editSessionId"] == id, "old-revision cancel discarded draft editing");
        editor(app, "edit_end", id, None).await?;
        assert_stable(app, draft).await?;
        ensure!(count(&fixture.final_requests) == finals && count(&fixture.polish) == polish,
            "old-revision cancel allowed late edit_end to start the operation");
    }
    let before = keyboard::validation_snapshot(app);
    editor(app, "edit_begin", "idle-editor", None).await?;
    click(app, "k3").await?;
    let after = keyboard::validation_snapshot(app);
    ensure!(after["editSessionId"] == "idle-editor" && after["text"] == before["text"]
        && after["revision"] == before["revision"] && after["dictationSelection"] == before["dictationSelection"],
        "idle K3 altered text, caret or unsaved editing");
    editor(app, "edit_end", "idle-editor", None).await?;
    if !fixture.platform_asr {
        fixture.hold_handshakes.store(true, Ordering::Release);
        hardware(app, None).await?;
        let connections = count(&fixture.connections);
        let finals = count(&fixture.final_requests);
        let pcm = count(&fixture.pcm);
        hardware(app, Some(true)).await?;
        wait_counter(&fixture.connections, connections + 1).await?;
        wait_state(app, |state| state["capturePreparing"] == true && state["audioState"]["state"] == "starting").await?;
        click(app, "k3").await?;
        fixture.hold_handshakes.store(false, Ordering::Release);
        hardware(app, Some(false)).await?;
        assert_stable(app, before["text"].as_str().unwrap()).await?;
        ensure!(count(&fixture.final_requests) == finals && count(&fixture.pcm) == pcm,
            "cancelled preparation opened capture or submitted recognition");
    }
    Ok(json!({"pending_capture_and_beautify_revoked":true,"unsaved_editor_lease_preserved":true,"late_ui_intents_rejected_after_save":true,"old_revision_cancel_precedes_late_edit_end":true,
        "idle_k3_preserves_body_revision_and_caret":true,"streaming_handshake_cancel_checked":!fixture.platform_asr}))
}
async fn cancellation_boundaries(app: &tauri::AppHandle, fixture: &Fixture) -> Result<Value> {
    let original = keyboard::validation_snapshot(app)["text"].as_str().unwrap().to_owned();
    fixture.hold_final.store(true, Ordering::Release);
    let finals = count(&fixture.final_requests);
    let attempts = count(&fixture.final_attempts);
    let polish = count(&fixture.polish);
    start(app, fixture, "本段不得写入").await?;
    click(app, "k2").await?;
    wait_counter(&fixture.final_requests, finals + 1).await?;
    ensure!(keyboard::validation_snapshot(app)["pendingAction"] == "beautify", "K2 failed to wait for ASR final");
    click(app, "k3").await?;
    hardware(app, Some(false)).await?;
    fixture.hold_final.store(false, Ordering::Release);
    wait_counter(&fixture.final_attempts, attempts + 1).await?;
    assert_stable(app, &original).await?;
    ensure!(count(&fixture.polish) == polish, "cancelled K2 was started after late ASR final");
    fixture.hold_polish_final.store(true, Ordering::Release);
    let attempts = count(&fixture.polish_attempts);
    click(app, "k2").await?;
    wait_state(app, |state| state["phase"] == "beautifying" && state["candidate"].as_str().is_some_and(|text| !text.is_empty())).await?;
    ensure!(keyboard::validation_snapshot(app)["text"] == original, "candidate replaced the committed body");
    click(app, "k3").await?;
    fixture.hold_polish_final.store(false, Ordering::Release);
    wait_counter(&fixture.polish_attempts, attempts + 1).await?;
    assert_stable(app, &original).await?;
    click(app, "k2").await?;
    settled(app, &format!("美化：{original}")).await?;
    Ok(json!({"finalizing_k3_cancels_queued_beautify":true,"late_asr_final_ignored":true,
        "candidate_cancel_keeps_original":true,"late_polish_finish_ignored":true,"beautify_retry_succeeds":true}))
}
async fn repeated_use(app: &tauri::AppHandle, fixture: &Fixture) -> Result<Value> {
    close(app).await?;
    let mut expected = String::new();
    let mut successes = 0;
    let mut cancelled_finals = 0;
    for round in 0..25 {
        let finals = count(&fixture.final_requests);
        let uploads = count(&fixture.uploads);
        let cancelled_request = start(app, fixture, "取消的录音不得提交").await?;
        hardware(app, Some(true)).await?;
        click(app, "k3").await?;
        hardware(app, Some(false)).await?;
        hardware(app, Some(false)).await?;
        settled(app, &expected).await?;
        ensure!(count(&fixture.final_requests) == finals && count(&fixture.uploads) == uploads,
            "active K3/up submitted an ASR request on round {round}");
        if round % 5 == 0 {
            fixture.hold_final.store(true, Ordering::Release);
            let attempts = count(&fixture.final_attempts);
            start(app, fixture, "取消收尾与等待中的旧按压").await?;
            hardware(app, Some(false)).await?;
            wait_counter(&fixture.final_requests, finals + 1).await?;
            hardware(app, Some(true)).await?;
            click(app, "k3").await?;
            fixture.hold_final.store(false, Ordering::Release);
            wait_counter(&fixture.final_attempts, attempts + 1).await?;
            // Keep the physical key down until after the old result arrives.
            // K3 must consume Waiting, rather than starting another recording.
            assert_stable(app, &expected).await?;
            hardware(app, Some(false)).await?;
            cancelled_finals += 1;
        }
        let text = format!("第{round}句。");
        start(app, fixture, &text).await?;
        if round == 0 {
            let current = keyboard::validation_snapshot(app);
            let replay = captured_action(app, "cancel", &cancelled_request).await;
            ensure!(replay.is_ok() == (current["documentId"] == cancelled_request["documentId"]),
                "old cancel did not enforce the current document identity");
            ensure!(keyboard::validation_snapshot(app)["phase"] == "recording", "duplicate old cancel stopped a later gesture");
        }
        hardware(app, Some(true)).await?;
        hardware(app, Some(false)).await?;
        hardware(app, Some(false)).await?;
        expected.push_str(&text);
        settled(app, &expected).await.with_context(|| format!("recovery after K3 round {round}"))?;
        successes += 1;
    }
    // A new hold without cancellation still waits for the previous final,
    // then starts automatically; cancellation must not remove this behavior.
    fixture.hold_final.store(true, Ordering::Release);
    let finals = count(&fixture.final_requests);
    start(app, fixture, "连续前句。").await?;
    hardware(app, Some(false)).await?;
    wait_counter(&fixture.final_requests, finals + 1).await?;
    hardware(app, Some(true)).await?;
    *fixture.asr_text.lock().unwrap() = "连续后句。".into();
    fixture.hold_final.store(false, Ordering::Release);
    expected.push_str("连续前句。");
    wait_state(app, |state| state["phase"] == "recording" && state["audioState"]["state"] == "capturing" && state["text"] == expected).await?;
    tokio::time::sleep(Duration::from_millis(75)).await;
    hardware(app, Some(false)).await?;
    expected.push_str("连续后句。");
    let final_state = settled(app, &expected).await?;
    ensure!(!fixture.upload_before_stop.load(Ordering::Acquire), "file ASR uploaded before positive microphone stop");
    Ok(json!({"rounds":25,"active_recordings_cancelled":25,"finalizing_waiting_holds_cancelled":cancelled_finals,
        "successful_segments":successes+2,"duplicate_edges_do_not_resubmit":true,"held_next_gesture_still_resumes":true,
        "platform_cancel_does_not_upload":fixture.platform_asr,"final_text":final_state["text"],"final_revision":final_state["revision"]}))
}
async fn caret_and_close(app: &tauri::AppHandle, fixture: &Fixture) -> Result<Value> {
    edit(app, "甲🙂乙", None).await?;
    editor(app, "edit_begin", "caret-insert", None).await?;
    editor(app, "edit_end", "caret-insert", Some(TextSelection { anchor: 3, focus: 3 })).await?;
    start(app, fixture, "插入").await?;
    hardware(app, Some(false)).await?;
    let inserted = settled(app, "甲🙂插入乙").await?;
    ensure!(inserted["dictationSelection"] == json!({"anchor":5,"focus":5}), "UTF-16 insertion caret was not preserved");
    edit(app, "保存后仍可关闭的正文", None).await?;
    captured_action(app, "close", &inserted).await?;
    wait_state(app, |state| idle(state) && state["visible"] == false && state["documentId"].is_null()).await?;
    fixture.hold_final.store(true, Ordering::Release);
    let finals = count(&fixture.final_requests);
    let attempts = count(&fixture.final_attempts);
    start(app, fixture, "关闭后迟到的旧结果").await?;
    hardware(app, Some(false)).await?;
    wait_counter(&fixture.final_requests, finals + 1).await?;
    let old_state = keyboard::validation_snapshot(app);
    let old_document = old_state["documentId"].clone();
    close(app).await?;
    let fresh = start(app, fixture, "全新正文").await?;
    ensure!(fresh["documentId"] != old_document && fresh["text"] == "", "closed draft leaked into new recording");
    ensure!(captured_action(app, "close", &old_state).await.is_err(),
        "stale close was accepted for a new document");
    fixture.hold_final.store(false, Ordering::Release);
    wait_counter(&fixture.final_attempts, attempts + 1).await?;
    tokio::time::sleep(Duration::from_millis(75)).await;
    ensure!(keyboard::validation_snapshot(app)["text"] == "", "late closed final wrote into the new document");
    hardware(app, Some(false)).await?;
    settled(app, "全新正文").await?;
    close(app).await?;
    Ok(json!({"utf16_caret_insertion":true,"close_discards_draft":true,"old_revision_close_allowed_same_document":true,"stale_close_rejected_new_document":true,"closed_late_final_isolated":true}))
}
async fn wait_transport_control(app: &tauri::AppHandle, rearm: bool) -> Result<u64> {
    let deadline = tokio::time::Instant::now() + Duration::from_millis(500);
    loop {
        match keyboard::validation_transport_control(app).map_err(anyhow::Error::msg)? {
            Some(nomifun_keyboard::service::Control::Rearm { generation }) if rearm => return Ok(generation),
            Some(nomifun_keyboard::service::Control::Policy { voice_key }) if !rearm && voice_key == "none" => return Ok(0),
            _ => {}
        }
        ensure!(tokio::time::Instant::now() < deadline,
            "actor admission command did not reach the existing serial worker before the next tick/status");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}
async fn update_recovery(app: &tauri::AppHandle, fixture: &Fixture) -> Result<Value> {
    action(app, "open").await?;
    edit(app, "更新前已定稿。", None).await?;
    let settings = keyboard::validation_snapshot(app)["settings"].clone();
    let mut expected = "更新前已定稿。".to_owned();
    let mut cases = Vec::new();
    let transport_owner = keyboard::validation_transport_owner(app).map_err(anyhow::Error::msg)?;
    ensure!(transport_owner["worker"].is_string() && transport_owner["lockHeld"] == true
        && transport_owner["endpointPublished"] == false, "isolated in-process serial owner is missing or published IPC");

    let original_settings: Settings = serde_json::from_value(settings.clone())?;
    for enabled in [false, true] {
        let mut unconfigured = original_settings.clone();
        unconfigured.enabled = enabled;
        unconfigured.asr_model = None;
        unconfigured.text_model = None;
        keyboard::validation_request(app, None, Some(unconfigured.clone())).await.map_err(anyhow::Error::msg)?;
        let pause = keyboard::prepare_update(app).await.map_err(anyhow::Error::msg)?
            .context("unconfigured actor did not admit maintenance")?;
        pause.cancel().await.map_err(anyhow::Error::msg)?;
        ensure!(keyboard::validation_snapshot(app)["settings"] == serde_json::to_value(unconfigured)?,
            "maintenance changed an unconfigured keyboard's preferences");
    }
    keyboard::validation_request(app, None, Some(original_settings)).await.map_err(anyhow::Error::msg)?;

    for scenario in ["cancel_after_ready", "cancel_during_native_stop", "native_stop_timeout", "external_native_stop_timeout"] {
        let final_requests = count(&fixture.final_requests);
        let uploads = count(&fixture.uploads);
        let previous_rearm = wait_transport_control(app, true).await?;
        let delayed_stop = scenario != "cancel_after_ready";
        keyboard::validation_hold_capture_stop(app, delayed_stop).await.map_err(anyhow::Error::msg)?;
        hardware(app, None).await?;
        start(app, fixture, "维护取消的半句话不能写入").await?;

        if scenario == "cancel_after_ready" {
            let pause = keyboard::prepare_update(app).await.map_err(anyhow::Error::msg)?
                .context("synthetic actor did not provide update guard")?;
            settled(app, &expected).await?;
            ensure!(keyboard::validation_update_paused(app), "ready preflight did not retain update admission");
            wait_transport_control(app, false).await?;
            // Keep the old physical key down during maintenance. Neither it nor
            // another press can revive the capture cancelled by this update.
            hardware(app, Some(true)).await?;
            assert_stable(app, &expected).await?;
            pause.cancel().await.map_err(anyhow::Error::msg)?;
        } else if scenario == "cancel_during_native_stop" {
            let copy = app.clone();
            let pending = tauri::async_runtime::spawn(async move { keyboard::prepare_update(&copy).await });
            wait_state(app, |state| state["audioState"]["state"] == "stopping").await?;
            ensure!(keyboard::validation_update_paused(app), "preflight did not hold admission while native stop was pending");
            wait_transport_control(app, false).await?;
            pending.abort();
            ensure!(pending.await.is_err(), "cancelled preflight unexpectedly completed");
            let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
            while keyboard::validation_update_paused(app) {
                ensure!(tokio::time::Instant::now() < deadline, "cancelled preflight left admission paused");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            ensure!(keyboard::validation_snapshot(app)["audioState"]["state"] == "stopping",
                "cancel claimed native capture released before its proof");
        } else {
            let copy = app.clone();
            let external = scenario == "external_native_stop_timeout";
            let pending = tauri::async_runtime::spawn(async move {
                if external { keyboard::prepare_external_maintenance(&copy).await }
                else { keyboard::prepare_update(&copy).await.map(|_| ()) }
            });
            wait_state(app, |state| state["audioState"]["state"] == "stopping").await?;
            wait_transport_control(app, false).await?;
            let failed = pending.await?.is_err();
            ensure!(failed, "update succeeded while native capture release was held");
            ensure!(!keyboard::validation_update_paused(app), "failed update left admission paused");
            ensure!(keyboard::validation_snapshot(app)["audioState"]["state"] == "stopping",
                "failed update lost the pending native capture owner");
        }

        let resumed_rearm = wait_transport_control(app, true).await?;
        ensure!(resumed_rearm > previous_rearm, "cancel reused the revoked Rearm generation");
        keyboard::validation_hold_capture_stop(app, false).await.map_err(anyhow::Error::msg)?;
        assert_stable(app, &expected).await?;
        ensure!(!keyboard::validation_update_paused(app), "cancel/failed update left admission paused");
        ensure!(count(&fixture.final_requests) == final_requests && count(&fixture.uploads) == uploads,
            "update stop waited for cloud recognition or uploaded its cancelled recording");
        ensure!(keyboard::validation_snapshot(app)["settings"] == settings, "update modified keyboard preferences");
        // A fresh user gesture must work after every cancellation/failure.
        hardware(app, None).await?;
        let text = format!("{scenario}后恢复。");
        start(app, fixture, &text).await?;
        hardware(app, Some(false)).await?;
        expected.push_str(&text);
        settled(app, &expected).await?;
        ensure!(keyboard::validation_transport_owner(app).map_err(anyhow::Error::msg)? == transport_owner,
            "cancelled update replaced or released the in-process serial owner");
        cases.push(json!({"scenario":scenario,"capture_recovered":true,"old_hold_not_resumed":true,
            "native_stop_proof_retained":true,"preferences_preserved":true,"same_transport_owner_retained":true,
            "none_reached_owner_before_tick_or_status":true,"fresh_rearm_after_cancel":true}));
    }

    keyboard::prepare_external_maintenance(app).await.map_err(anyhow::Error::msg)?;
    hardware(app, Some(true)).await?;
    assert_stable(app, &expected).await?;
    ensure!(keyboard::validation_update_paused(app), "committed handoff resumed the replaced runtime");
    let stopped_owner = keyboard::validation_transport_owner(app).map_err(anyhow::Error::msg)?;
    ensure!(stopped_owner["worker"].is_null() && stopped_owner["lockHeld"] == false
        && stopped_owner["endpointPublished"] == false, "native shutdown did not join/release the serial owner");
    Ok(json!({"cases":cases,"successful_handoff_keeps_old_runtime_paused":true,
        "in_process_transport_owner":transport_owner,"serial_owner_joined_after_native_stop":true,
        "serial_lock_released_after_shutdown":true,"no_daemon_endpoint_published":true,
        "cloud_completion_not_required":true,"automatic_recording_not_resumed":true,
        "disabled_and_unconfigured_keyboard_admit_maintenance":true,
        "scope":"production actor with synthetic capture; package file replacement is a separate witness"}))
}
async fn local_microphone_checks(app: &tauri::AppHandle, settings: &Settings, fixture: &Fixture) -> Result<Value> {
    let mut unconfigured = settings.clone();
    unconfigured.enabled = false;
    unconfigured.k4_enabled = false;
    unconfigured.asr_model = None;
    keyboard::validation_request(app, None, Some(unconfigured.clone())).await.map_err(anyhow::Error::msg)?;
    let local_before = keyboard::validation_snapshot(app);
    let provider_before = [count(&fixture.connections), count(&fixture.uploads), count(&fixture.final_requests), count(&fixture.pcm)];
    keyboard::validation_microphone_test(app, "microphone_test_start", "local-initial", None)
        .await.map_err(anyhow::Error::msg).context("local test must work without ASR, USB or K4")?;
    let local = wait_state(app, |state| state["microphoneTest"]["state"] == "completed").await?;
    ensure!(local["microphoneTest"]["stopped"] == true && local["microphoneTest"]["callbackObserved"] == true
        && local["microphoneTest"]["samples"].as_u64().unwrap_or(0) > 0, "local test lacks callback/sample/native-stop proof");
    for field in ["documentId", "revision", "text", "phase", "editSessionId", "settings"] {
        ensure!(local[field] == local_before[field], "local test changed {field}");
    }
    ensure!(provider_before == [count(&fixture.connections), count(&fixture.uploads), count(&fixture.final_requests), count(&fixture.pcm)], "local-only test called a provider");
    action(app, "open").await?;
    edit(app, "本地检查保留正文🙂", None).await?;
    editor(app, "edit_begin", "local-editor", None).await?;
    let document_before = keyboard::validation_snapshot(app);
    keyboard::validation_hold_capture_stop(app, true).await.map_err(anyhow::Error::msg)?;
    keyboard::validation_microphone_test(app, "microphone_test_start", "local-held", Some("draft-test-device".into()))
        .await.map_err(anyhow::Error::msg)?;
    wait_state(app, |state| state["microphoneTest"]["state"] == "listening").await?;
    ensure!(keyboard::validation_microphone_test(app, "microphone_test_start", "local-overlap", None).await.is_err(),
        "a second local test acquired the same native slot");
    keyboard::validation_microphone_test(app, "microphone_test_stop", "expired-test", None).await.map_err(anyhow::Error::msg)?;
    ensure!(keyboard::validation_snapshot(app)["microphoneTest"]["state"] == "listening", "stale test ID stopped a new owner");
    keyboard::validation_microphone_test(app, "microphone_test_stop", "local-held", None).await.map_err(anyhow::Error::msg)?;
    let stopping = wait_state(app, |state| state["microphoneTest"]["state"] == "stopping").await?;
    ensure!(stopping["microphoneTest"]["stopped"] == false, "stop request forged native stop proof");
    ensure!(keyboard::validation_microphone_test(app, "microphone_test_start", "local-before-proof", None).await.is_err(),
        "local start bypassed the pending native owner");
    keyboard::validation_late_capture_callback(app).await.map_err(anyhow::Error::msg)?;
    keyboard::validation_hold_capture_stop(app, false).await.map_err(anyhow::Error::msg)?;
    let cancelled = wait_state(app, |state| state["microphoneTest"]["state"] == "cancelled" && state["microphoneTest"]["stopped"] == true).await?;
    for field in ["documentId", "revision", "text", "phase", "editSessionId", "settings"] {
        ensure!(cancelled[field] == document_before[field], "local cancellation changed {field}");
    }
    ensure!(cancelled["microphoneTest"]["deviceId"] == "draft-test-device" && cancelled["settings"]["audioDeviceId"].is_null(),
        "local test saved its temporary draft device");
    editor(app, "edit_cancel", "local-editor", None).await?;

    keyboard::validation_microphone_test(app, "microphone_test_start", "local-suspend", None).await.map_err(anyhow::Error::msg)?;
    keyboard::validation_suspend(app, true).await.map_err(anyhow::Error::msg)?;
    wait_state(app, |state| state["microphoneTest"]["state"] == "cancelled" && state["microphoneTest"]["stopped"] == true).await?;
    ensure!(keyboard::validation_microphone_test(app, "microphone_test_start", "local-while-locked", None).await.is_err(),
        "local test started in a suspended session");
    keyboard::validation_suspend(app, false).await.map_err(anyhow::Error::msg)?;

    keyboard::validation_microphone_test(app, "microphone_test_start", "local-update", None).await.map_err(anyhow::Error::msg)?;
    let pause = keyboard::prepare_update(app).await.map_err(anyhow::Error::msg)?.context("local test did not stop for update")?;
    ensure!(keyboard::validation_snapshot(app)["microphoneTest"]["stopped"] == true, "update accepted before local stop proof");
    pause.cancel().await.map_err(anyhow::Error::msg)?;

    unconfigured.enabled = true;
    unconfigured.k4_enabled = true;
    keyboard::validation_request(app, None, Some(unconfigured)).await.map_err(anyhow::Error::msg)?;
    hardware(app, None).await?;
    keyboard::validation_microphone_test(app, "microphone_test_start", "local-k4", None).await.map_err(anyhow::Error::msg)?;
    wait_state(app, |state| state["microphoneTest"]["state"] == "listening").await?;
    let during_local = keyboard::validation_snapshot(app);
    hardware(app, Some(true)).await?;
    ensure!(keyboard::validation_snapshot(app)["phase"] == "idle" && keyboard::validation_snapshot(app)["operationId"] == during_local["operationId"],
        "K4 created a second owner during local test");
    keyboard::validation_microphone_test(app, "microphone_test_stop", "local-k4", None).await.map_err(anyhow::Error::msg)?;
    wait_state(app, |state| state["microphoneTest"]["stopped"] == true).await?;
    hardware(app, Some(false)).await?;
    hardware(app, Some(true)).await?;
    let missing = wait_state(app, |state| state["visible"] == true && state["error"].as_str().is_some_and(|error| error.contains("ASR"))).await?;
    ensure!(missing["audioState"]["state"] == "stopped" && missing["phase"] == "idle", "missing ASR opened native capture");
    hardware(app, Some(false)).await?;
    ensure!(provider_before == [count(&fixture.connections), count(&fixture.uploads), count(&fixture.final_requests), count(&fixture.pcm)],
        "local-only lifecycle or missing-model K4 called a provider");
    let provider_after = [count(&fixture.connections), count(&fixture.uploads), count(&fixture.final_requests), count(&fixture.pcm)];
    close(app).await?;
    let mut configured = settings.clone();
    configured.enabled = true;
    configured.k4_enabled = true;
    keyboard::validation_request(app, None, Some(configured)).await.map_err(anyhow::Error::msg)?;
    keyboard::validation_arm_prewarm(app).await.map_err(anyhow::Error::msg)?;
    if !fixture.platform_asr {
        wait_counter(&fixture.connections, 1).await?;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        while !keyboard::validation_asr_prepared(app).await.map_err(anyhow::Error::msg)? {
            ensure!(tokio::time::Instant::now() < deadline, "Actor did not accept idle ASR prewarm");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
    let prewarm_before = [count(&fixture.connections), count(&fixture.uploads), count(&fixture.final_requests), count(&fixture.pcm)];
    let prewarm_document = keyboard::validation_snapshot(app);
    keyboard::validation_microphone_test(app, "microphone_test_start", "local-with-prewarm", None).await.map_err(anyhow::Error::msg)?;
    wait_state(app, |state| state["microphoneTest"]["state"] == "completed").await?;
    tokio::time::sleep(Duration::from_millis(1200)).await; // include at least one normal Actor Tick
    let after_prewarm = keyboard::validation_snapshot(app);
    let prewarm_after = [count(&fixture.connections), count(&fixture.uploads), count(&fixture.final_requests), count(&fixture.pcm)];
    ensure!(prewarm_before == prewarm_after, "local test cancelled prewarm and caused a new provider request");
    for field in ["documentId", "revision", "text", "phase", "editSessionId", "settings"] {
        ensure!(after_prewarm[field] == prewarm_document[field], "prewarmed local test changed {field}");
    }
    Ok(json!({"without_asr_usb_or_k4":true,"five_second_auto_stop":true,"callback_and_pcm_evidence":local["microphoneTest"],
        "no_provider_calls":true,"provider_counts_before":provider_before,
        "provider_counts_after":provider_after,"prewarm_preserved_without_provider_reconnect":true,
        "prewarm_counts_before":prewarm_before,"prewarm_counts_after":prewarm_after,
        "document_settings_and_edit_lease_unchanged":true,"draft_device_not_saved":true,"stale_test_id_ignored":true,
        "mutual_exclusion":true,"stop_proof_required_before_new_owner":true,"late_cancelled_callback_does_not_fail":true,"suspension_cancels_and_blocks":true,
        "update_waits_for_local_stop":true,"k4_missing_model_visible_without_recording":true}))
}
async fn validate(app: &tauri::AppHandle, settings: Settings, fixture: &Fixture) -> Result<Value> {
    let local_microphone = local_microphone_checks(app, &settings, fixture).await.context("local microphone checks")?;
    keyboard::validation_request(app, None, Some(settings)).await.map_err(anyhow::Error::msg)?;
    hardware(app, None).await?;
    let editor = preparation_and_editor(app, fixture).await.context("preparation/editor cancellation")?;
    let boundaries = cancellation_boundaries(app, fixture).await.context("ASR/polish cancellation")?;
    let heavy = repeated_use(app, fixture).await.context("repeated K4/K3 use")?;
    let caret_close = caret_and_close(app, fixture).await.context("caret/close")?;
    let update_recovery = update_recovery(app, fixture).await.context("update pause/cancel/failure recovery")?;
    Ok(json!({"passed":true,"asr_mode":if fixture.platform_asr{"platform"}else{"streaming"},
        "audio_source":"explicit synthetic PCM fixture; no OS microphone","real_os_microphone":false,
        "provider_requests_loopback_only":true,"local_microphone":local_microphone,"editor":editor,"cancellation":boundaries,"heavy_use":heavy,
        "caret_and_close":caret_close,"update_recovery":update_recovery,
        "pcm_samples":count(&fixture.pcm),"uploads":count(&fixture.uploads),
        "asr_final_requests":count(&fixture.final_requests),"polish_requests":count(&fixture.polish)}))
}
#[derive(Debug)]
struct Options { platform_asr: bool, output: PathBuf }
fn parse(arguments: impl IntoIterator<Item=String>) -> Result<Options> {
    let arguments: Vec<_> = arguments.into_iter().collect();
    ensure!(arguments.len() == 2, "required: --synthetic-audio OR --platform-asr-fixture, then an absolute new report path");
    let platform_asr = match arguments[0].as_str() {
        "--synthetic-audio" => false,
        "--platform-asr-fixture" => true,
        _ => anyhow::bail!("explicit synthetic audio mode required; no real microphone mode exists"),
    };
    let output = PathBuf::from(&arguments[1]);
    ensure!(output.is_absolute() && !output.exists(), "pass an absolute, new private report path");
    Ok(Options { platform_asr, output })
}
fn main() -> Result<()> {
    let options = parse(std::env::args().skip(1))?;
    let root = tempfile::tempdir()?;
    std::fs::write(root.path().join(".synthetic-audio-fixture"), "local synthetic PCM only")?;
    std::fs::write(root.path().join(".native-keyboard-validation"), "isolated-native-keyboard-v1")?;
    let runtime = tokio::runtime::Runtime::new()?;
    let (service, settings, fixture) = runtime.block_on(fixtures(options.platform_asr, &root.path().join("provider-fixture.sqlite")))?;
    tauri::async_runtime::set(runtime.handle().clone());
    let result = Arc::new(Mutex::new(None::<Result<Value, String>>));
    let completed = result.clone();
    let root_path = root.path().to_path_buf();
    let app = tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![keyboard::keyboard_get_state, keyboard::keyboard_action])
        .setup(move |app| {
            *fixture.app.lock().unwrap() = Some(app.handle().clone());
            keyboard::validation_start(app.handle().clone(), service.clone(), root_path)?;
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let outcome = validate(&handle, settings, &fixture).await.map_err(|error| format!("{error:#}"));
                *completed.lock().unwrap() = Some(outcome);
                handle.exit(0);
            });
            Ok(())
        }).build(tauri::generate_context!())?;
    app.run_return(|_, _| {});
    let outcome = result.lock().unwrap().take().context("fixture exited without a result")?;
    let mut report = match &outcome {
        Ok(report) => report.clone(),
        Err(error) => json!({"passed":false,"error":error,"asr_mode":if options.platform_asr{"platform"}else{"streaming"},
            "audio_source":"explicit synthetic PCM fixture; no OS microphone","real_os_microphone":false}),
    };
    report["frontend_build_id"] = json!(option_env!("NOMIFUN_FRONTEND_BUILD_ID"));
    std::fs::write(&options.output, serde_json::to_vec_pretty(&report)?)?;
    println!("{report}");
    outcome.map_err(anyhow::Error::msg)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_explicit_synthetic_mode_and_new_absolute_report_are_required() {
        for args in [vec![], vec!["--synthetic-audio"], vec!["--expect-no-input", "out.json"],
            vec!["--synthetic-audio", "relative.json"], vec!["--synthetic-audio", "--platform-asr-fixture", "out.json"]] {
            assert!(parse(args.into_iter().map(str::to_owned)).is_err());
        }
        let directory = tempfile::tempdir().unwrap();
        for mode in ["--synthetic-audio", "--platform-asr-fixture"] {
            let path = directory.path().join("fresh.json");
            let options = parse([mode.into(), path.to_string_lossy().into_owned()]).unwrap();
            assert_eq!(options.platform_asr, mode == "--platform-asr-fixture");
            std::fs::write(&path, "existing").unwrap();
            assert!(parse([mode.into(), path.to_string_lossy().into_owned()]).is_err());
            std::fs::remove_file(path).unwrap();
        }
    }
}
