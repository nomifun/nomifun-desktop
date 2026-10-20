//! Incremental speech media without conversation or work authority.
//!
//! A full-upload ASR endpoint (including SSE transcript responses) is batch
//! input. Incremental input requires an explicitly selected session protocol.
//! HTTP TTS below pulls actual PCM bytes before EOF; it never synthesizes a
//! complete asset and then relabels slices as streaming.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt, stream::{self, BoxStream}};
use nomifun_api_types::ModelTask;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tokio_tungstenite::{connect_async_tls_with_config, tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig}};

use crate::{AuthScheme, InvokeError, InvokeErrorKind, ModelInvokeService, ModelRef, ResolvedCall, ResolvedTaskConfig, TtsRequest};
use crate::types::TaskRequest;

pub const DEEPGRAM_INCREMENTAL_ASR_PROTOCOL: &str = "deepgram.listen_stream";
const PCM_RATE: u32 = 24_000;
const PCM_CHUNK_SAMPLES: usize = 1_920; // 80 ms; independent of HTTP packet size.
const MAX_NETWORK_CHUNK_BYTES: usize = 256 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 64 * 1024;
const MAX_UTTERANCE: Duration = Duration::from_secs(120);
const MEDIA_AGE: Duration = Duration::from_millis(750);
const FINAL_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SpeechProtocolCapabilities {
    pub incremental_asr: bool,
    pub streaming_tts: bool,
}

/// Protocol identity is the only discovery authority. Neither provider names
/// nor an SSE response imply incremental audio input or PCM output.
pub fn protocol_capabilities(protocol: &str) -> SpeechProtocolCapabilities {
    SpeechProtocolCapabilities {
        incremental_asr: protocol == DEEPGRAM_INCREMENTAL_ASR_PROTOCOL,
        streaming_tts: protocol == "openai.audio_speech",
    }
}

/// Dedicated speech sessions are registered separately from one-shot and
/// speech-to-speech conversational adapters. Their owning executor remains
/// ModelInvokeService; they cannot be dispatched through `invoke`.
pub(crate) fn is_registered_session_protocol(protocol: &str) -> bool {
    protocol == DEEPGRAM_INCREMENTAL_ASR_PROTOCOL
}

pub struct PcmAudioStream {
    pub sample_rate: u32,
    pub audio: BoxStream<'static, Result<Vec<i16>, InvokeError>>,
}

impl ModelInvokeService {
    pub async fn speech_protocol_capabilities(
        &self,
        model: &ModelRef,
        task: ModelTask,
    ) -> Result<SpeechProtocolCapabilities, InvokeError> {
        let config = self.resolve_task_config(model, task).await?;
        Ok(protocol_capabilities(&config.protocol))
    }

    /// None means the exact configured protocol requires batch delivery.
    /// Configuration/HTTP failures are errors, never silent batch retries.
    pub async fn open_streaming_synthesis(
        &self,
        model: &ModelRef,
        mut request: TtsRequest,
    ) -> Result<Option<PcmAudioStream>, InvokeError> {
        request.format = Some("pcm".into());
        let (call, _) = self.resolve(model, ModelTask::SpeechSynthesis, TaskRequest::SpeechSynthesis(request)).await?;
        if !protocol_capabilities(&call.protocol).streaming_tts {
            return Ok(None);
        }
        open_pcm_response(&self.http, &call).await.map(Some)
    }

    pub async fn open_streaming_transcription(
        &self,
        model: &ModelRef,
        sample_rate: u32,
        language: Option<String>,
    ) -> Result<Option<TranscriptionSession>, InvokeError> {
        let config = self.resolve_task_config(model, ModelTask::SpeechRecognition).await?;
        if !protocol_capabilities(&config.protocol).incremental_asr {
            return Ok(None);
        }
        create_transcription_session(config, sample_rate, language).await.map(Some)
    }
}

async fn open_pcm_response(http: &reqwest::Client, call: &ResolvedCall) -> Result<PcmAudioStream, InvokeError> {
    let TaskRequest::SpeechSynthesis(request) = &call.request else {
        return Err(InvokeError::config("incremental PCM requires a speech synthesis request"));
    };
    let mut body = crate::adapters::openai_audio::speech_body(call, request)?;
    if body.get("sample_rate").is_some_and(|rate|rate.as_u64()!=Some(PCM_RATE as u64)) {
        return Err(InvokeError::new(InvokeErrorKind::InvalidParams,"OpenAI streaming PCM requires its 24000 Hz mono format"));
    }
    body["response_format"] = json!("pcm");
    // Provider options cannot change the delivery framing behind the PCM port.
    // Audio is the protocol default, including older TTS models which do not
    // accept the newer `stream_format` parameter.
    body.as_object_mut().expect("speech body is an object").remove("stream_format");
    let response = crate::transport::post_json(http, &call.endpoint_url()?, Duration::from_secs(30), &call.connection.auth, &body).await?;
    if !response.status().is_success() {
        return Err(crate::transport::error_from_response(response).await);
    }
    let content_type = response.headers().get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok()).unwrap_or_default()
        .split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    if !matches!(content_type.as_str(), "audio/pcm" | "audio/raw" | "application/octet-stream") {
        return Err(InvokeError::parse("streaming PCM synthesis returned an incompatible Content-Type"));
    }
    if response.content_length().is_some_and(|size| size > crate::transport::MAX_ARTIFACT_BYTES) {
        return Err(InvokeError::parse("streaming speech exceeded its audio byte budget"));
    }
    struct State {
        network: BoxStream<'static, Result<Vec<u8>, InvokeError>>,
        pending: VecDeque<u8>,
        total: u64,
        emitted: bool,
        eof: bool,
    }
    let network = response.bytes_stream().map(|result| result.map(|bytes| bytes.to_vec())
        .map_err(|_| InvokeError::new(InvokeErrorKind::Network, "streaming speech response interrupted"))).boxed();
    let audio = stream::try_unfold(State { network, pending: VecDeque::new(), total: 0, emitted: false, eof: false }, |mut state| async move {
        loop {
            if state.pending.len() >= 2 {
                let samples = (state.pending.len() / 2).min(PCM_CHUNK_SAMPLES);
                let mut pcm = Vec::with_capacity(samples);
                for _ in 0..samples {
                    let low = state.pending.pop_front().expect("sample byte present");
                    let high = state.pending.pop_front().expect("sample byte present");
                    pcm.push(i16::from_le_bytes([low, high]));
                }
                state.emitted = true;
                return Ok(Some((pcm, state)));
            }
            if state.eof {
                if !state.pending.is_empty() {
                    return Err(InvokeError::parse("streaming PCM ended in a partial sample"));
                }
                if !state.emitted {
                    return Err(InvokeError::parse("streaming speech produced no samples"));
                }
                return Ok(None);
            }
            match state.network.next().await {
                Some(Ok(bytes)) => {
                    if state.total == 0 && !bytes.is_empty() {
                        crate::adapters::audio_payload::validate_audio_payload(&bytes, Some("audio/pcm"), "streaming PCM")?;
                        if bytes.starts_with(b"RIFF") || bytes.starts_with(b"ID3") || bytes.starts_with(b"OggS") || bytes.starts_with(b"fLaC") {
                            return Err(InvokeError::parse("streaming PCM returned an encoded audio container"));
                        }
                    }
                    state.total = state.total.saturating_add(bytes.len() as u64);
                    if bytes.len() > MAX_NETWORK_CHUNK_BYTES || state.total > crate::transport::MAX_ARTIFACT_BYTES {
                        return Err(InvokeError::parse("streaming speech exceeded its audio byte budget"));
                    }
                    state.pending.extend(bytes);
                }
                Some(Err(error)) => return Err(error),
                None => state.eof = true,
            }
        }
    }).boxed();
    Ok(PcmAudioStream { sample_rate: PCM_RATE, audio })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TranscriptEvent {
    Interim(String),
    /// A provider-final span, before the physical utterance has necessarily
    /// ended. Interim spans are never appended to the admitted transcript.
    FinalSegment(String),
    Finished(String),
}

struct InputPcm {
    samples: Vec<i16>,
    captured_at: Instant,
}

/// One physical utterance and one provider connection. Input and transcript
/// queues are bounded; finalization joins the exact worker. Dropping cancels
/// local ownership and never allows this transcript to join a later request.
pub struct TranscriptionSession {
    input: Option<mpsc::Sender<InputPcm>>,
    events: mpsc::Receiver<Result<TranscriptEvent, InvokeError>>,
    worker: Option<JoinHandle<()>>,
    sample_rate: u32,
}

impl TranscriptionSession {
    pub fn try_send(&self, samples: Vec<i16>) -> Result<(), InvokeError> {
        if samples.is_empty() || samples.len() > self.sample_rate as usize * 120 / 1000 {
            return Err(InvokeError::new(InvokeErrorKind::InvalidParams, "incremental ASR frames must contain 1-120 ms of mono PCM"));
        }
        self.input.as_ref().ok_or_else(|| InvokeError::config("transcription input is already finished"))?
            .try_send(InputPcm { samples, captured_at: Instant::now() })
            .map_err(|_| InvokeError::new(InvokeErrorKind::Network, "incremental ASR input queue is full or closed"))
    }

    pub async fn recv(&mut self) -> Option<Result<TranscriptEvent, InvokeError>> {
        self.events.recv().await
    }

    /// Finish audio admission, drain to the provider's explicit stream close,
    /// and join. A socket disappearing without final metadata is not success.
    pub async fn finish(mut self) -> Result<String, InvokeError> {
        self.finish_result().await
    }

    /// Borrowed finalization lets a product select cancellation and then call
    /// `cancel_and_wait` on the same owned session without detaching its worker.
    pub async fn finish_result(&mut self) -> Result<String, InvokeError> {
        self.input.take();
        let result = tokio::time::timeout(FINAL_TIMEOUT + MEDIA_AGE, async {
            while let Some(event) = self.events.recv().await {
                match event? {
                    TranscriptEvent::Finished(text) => return Ok(text),
                    TranscriptEvent::Interim(_) | TranscriptEvent::FinalSegment(_) => {}
                }
            }
            Err(InvokeError::new(InvokeErrorKind::Network, "ASR stream ended without finalization"))
        }).await.unwrap_or_else(|_| Err(InvokeError::new(InvokeErrorKind::Timeout, "ASR finalization exceeded its response budget")));
        self.stop_worker().await;
        result
    }

    pub async fn cancel_and_wait(mut self) {
        self.input.take();
        self.stop_worker().await;
    }

    async fn stop_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            worker.abort();
            let _ = worker.await;
        }
    }
}

impl Drop for TranscriptionSession {
    fn drop(&mut self) {
        self.input.take();
        if let Some(worker) = self.worker.take() {
            worker.abort();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move { let _ = worker.await; });
            }
        }
    }
}

/// Bind a private, exact resolver lease to one media session. Useful to host
/// adapters which already resolved a frozen task configuration; no endpoint
/// or provider is guessed from a catalog name.
pub async fn create_transcription_session(config: ResolvedTaskConfig, sample_rate: u32, language: Option<String>) -> Result<TranscriptionSession, InvokeError> {
    validate_transcription_configuration(&config)?;
    if !(8_000..=48_000).contains(&sample_rate) {
        return Err(InvokeError::new(InvokeErrorKind::InvalidParams, "ASR PCM sample rate must be between 8000 and 48000 Hz"));
    }
    open_deepgram_transcription(config,sample_rate,language).await
}

pub(crate) fn validate_transcription_configuration(config:&ResolvedTaskConfig)->Result<(),InvokeError> {
    if config.protocol != DEEPGRAM_INCREMENTAL_ASR_PROTOCOL || config.task != ModelTask::SpeechRecognition {
        return Err(InvokeError::new(InvokeErrorKind::UnsupportedTask, "the selected protocol has no incremental transcription session"));
    }
    if !matches!(config.connection.auth.scheme, AuthScheme::TokenHeader | AuthScheme::Bearer) {
        return Err(InvokeError::config("Deepgram streaming ASR requires token or bearer authentication"));
    }
    config.connection.auth.validate()?;
    let template = config.transport.realtime_endpoint.as_deref()
        .ok_or_else(|| InvokeError::config("streaming ASR has no session endpoint"))?;
    let endpoint = crate::manifest::expand_protocol_endpoint_template(&config.protocol, config.task, "realtime_endpoint", template, &config.model)?;
    super::transport::endpoint(&config.connection,&endpoint,true,config.transport.allow_cross_origin_credentials)?;
    Ok(())
}

async fn open_deepgram_transcription(config:ResolvedTaskConfig,sample_rate:u32,language:Option<String>)->Result<TranscriptionSession,InvokeError> {
    let template=config.transport.realtime_endpoint.as_deref().expect("validated session endpoint");
    let endpoint=crate::manifest::expand_protocol_endpoint_template(&config.protocol,config.task,"realtime_endpoint",template,&config.model)?;
    let mut url = super::transport::endpoint(&config.connection, &endpoint, true, config.transport.allow_cross_origin_credentials)?;
    let mut fields = crate::adapters::scalar_request_fields(&config.provider_params, &json!({}))?;
    for key in ["model", "encoding", "sample_rate", "channels", "interim_results", "detect_language"] {
        fields.remove(key);
    }
    fields.insert("model".into(), config.model);
    fields.insert("encoding".into(), "linear16".into());
    fields.insert("sample_rate".into(), sample_rate.to_string());
    fields.insert("channels".into(), "1".into());
    fields.insert("interim_results".into(), "true".into());
    if let Some(language) = language {
        fields.insert("language".into(), language);
    }
    // Endpoint query is part of the selected protocol; typed PCM identity
    // overrides conflicting values rather than being appended twice.
    let mut query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    query.extend(fields);
    url.query_pairs_mut().clear().extend_pairs(query);
    let mut socket = None;
    let mut rejected=None;
    let prefix = match config.connection.auth.scheme {
        AuthScheme::TokenHeader => "Token",
        AuthScheme::Bearer => "Bearer",
        _ => return Err(InvokeError::config("Deepgram streaming ASR requires token or bearer authentication")),
    };
    for secret in config.connection.auth.secrets() {
        let mut request = url.as_str().into_client_request().map_err(|_| InvokeError::config("invalid ASR WebSocket endpoint"))?;
        let mut authorization = tokio_tungstenite::tungstenite::http::HeaderValue::from_str(&format!("{prefix} {secret}"))
            .map_err(|_| InvokeError::config("invalid ASR authentication header"))?;
        authorization.set_sensitive(true);
        request.headers_mut().insert(tokio_tungstenite::tungstenite::http::header::AUTHORIZATION, authorization);
        let wire = WebSocketConfig::default().max_message_size(Some(256 * 1024)).max_frame_size(Some(256 * 1024));
        match tokio::time::timeout(Duration::from_secs(8), connect_async_tls_with_config(request, Some(wire), false, None)).await {
            Ok(Ok((connected, _))) => { socket = Some(connected); break; }
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) if matches!(response.status().as_u16(), 401 | 403 | 429) => {
                let status=response.status().as_u16();
                rejected=Some(InvokeError::new(if status==429 {InvokeErrorKind::RateLimited} else {InvokeErrorKind::Auth},
                    format!("ASR WebSocket handshake rejected with HTTP {status}")).with_http_status(status));
                continue;
            },
            Ok(Err(_)) => return Err(InvokeError::new(InvokeErrorKind::Network, "ASR WebSocket handshake failed")),
            Err(_) => return Err(InvokeError::new(InvokeErrorKind::Timeout, "ASR WebSocket handshake timed out")),
        }
    }
    let socket = socket.ok_or_else(|| rejected.unwrap_or_else(||InvokeError::config("ASR connection has no usable credentials")))?;
    let (input, input_rx) = mpsc::channel(8);
    let (events_tx, events) = mpsc::channel(32);
    let worker = tokio::spawn(async move {
        if let Err(error) = transcribe_socket(socket, input_rx, &events_tx).await {
            let _ = events_tx.send(Err(error)).await;
        }
    });
    Ok(TranscriptionSession { input: Some(input), events, worker: Some(worker), sample_rate })
}

async fn transcribe_socket(
    mut socket: super::transport::Socket,
    mut input: mpsc::Receiver<InputPcm>,
    events: &mpsc::Sender<Result<TranscriptEvent, InvokeError>>,
) -> Result<(), InvokeError> {
    let mut finalizing = false;
    let mut finalized = false;
    let mut deadline = Instant::now() + MAX_UTTERANCE;
    let mut final_spans = BTreeMap::<u64, String>::new();
    loop {
        tokio::select! { biased;
            _ = events.closed() => return Ok(()),
            _ = tokio::time::sleep_until(deadline) => return Err(InvokeError::new(InvokeErrorKind::Timeout, "ASR utterance or finalization exceeded its media budget")),
            frame = input.recv(), if !finalizing => {
                let message = if let Some(frame) = frame {
                    if frame.captured_at.elapsed() > MEDIA_AGE {
                        return Err(InvokeError::new(InvokeErrorKind::Timeout, "ASR input frame exceeded its media age budget"));
                    }
                    let bytes: Vec<u8> = frame.samples.into_iter().flat_map(i16::to_le_bytes).collect();
                    Message::Binary(bytes.into())
                } else {
                    finalizing = true;
                    deadline = Instant::now() + FINAL_TIMEOUT;
                    Message::Text(json!({"type":"CloseStream"}).to_string().into())
                };
                tokio::time::timeout(MEDIA_AGE, socket.send(message)).await
                    .map_err(|_| InvokeError::new(InvokeErrorKind::Timeout, "ASR audio write exceeded its media budget"))?
                    .map_err(|_| InvokeError::new(InvokeErrorKind::Network, "ASR audio write failed"))?;
            }
            message = socket.next() => {
                let value = match message {
                    Some(Ok(Message::Text(text))) => serde_json::from_str::<Value>(&text)
                        .map_err(|_| InvokeError::parse("ASR provider sent malformed JSON"))?,
                    Some(Ok(Message::Ping(payload))) => {
                        tokio::time::timeout(MEDIA_AGE, socket.send(Message::Pong(payload))).await
                            .map_err(|_| InvokeError::new(InvokeErrorKind::Timeout, "ASR pong exceeded its media budget"))?
                            .map_err(|_| InvokeError::new(InvokeErrorKind::Network, "ASR pong failed"))?;
                        continue;
                    }
                    Some(Ok(Message::Close(_))) | None if finalizing && finalized => {
                        let text = final_spans.into_values().filter(|text| !text.is_empty()).collect::<Vec<_>>().join(" ");
                        events.try_send(Ok(TranscriptEvent::Finished(text)))
                            .map_err(|_| InvokeError::new(InvokeErrorKind::Network, "ASR transcript queue exceeded its budget"))?;
                        return Ok(());
                    }
                    Some(Ok(Message::Pong(_))) => continue,
                    _ => return Err(InvokeError::new(InvokeErrorKind::Network, "ASR stream ended before confirmed finalization")),
                };
                match value.get("type").and_then(Value::as_str) {
                    Some("Results") => {
                        let text = value["channel"]["alternatives"][0]["transcript"].as_str()
                            .ok_or_else(|| InvokeError::parse("ASR result requires a transcript"))?.to_owned();
                        if text.len() > MAX_TRANSCRIPT_BYTES {
                            return Err(InvokeError::parse("ASR transcript exceeded its text budget"));
                        }
                        let event = if value.get("is_final").and_then(Value::as_bool) == Some(true) {
                            let start = value.get("start").and_then(Value::as_f64)
                                .filter(|start| start.is_finite() && *start >= 0.0)
                                .ok_or_else(|| InvokeError::parse("ASR final result requires a valid source offset"))?;
                            final_spans.insert((start * 1_000_000.0).round() as u64, text.clone());
                            if final_spans.values().map(String::len).sum::<usize>() > MAX_TRANSCRIPT_BYTES {
                                return Err(InvokeError::parse("ASR utterance exceeded its text budget"));
                            }
                            TranscriptEvent::FinalSegment(text)
                        } else { TranscriptEvent::Interim(text) };
                        // Interim saturation may be discarded; final spans are
                        // retained in this worker and finish returns the exact
                        // provider-final transcript, never the latest draft.
                        match events.try_send(Ok(event)) {
                            Ok(()) | Err(mpsc::error::TrySendError::Full(Ok(TranscriptEvent::Interim(_)))) => {},
                            Err(_) => return Err(InvokeError::new(InvokeErrorKind::Network, "ASR transcript queue exceeded its budget")),
                        }
                    }
                    Some("Metadata") if finalizing => finalized = true,
                    Some("Error") => return Err(InvokeError::new(InvokeErrorKind::ProviderError, "ASR provider rejected the audio stream")),
                    _ => {},
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AuthMaterial, ResolvedConnection, ResolvedTaskTransport};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::oneshot;
    use tokio_tungstenite::accept_hdr_async;

    async fn read_request(wire:&mut tokio::net::TcpStream)->Vec<u8> {
        let mut request=Vec::new();
        loop {
            let mut chunk=[0;4096];
            let read=wire.read(&mut chunk).await.unwrap();
            assert!(read>0,"request must include its declared body");
            request.extend_from_slice(&chunk[..read]);
            assert!(request.len()<16*1024);
            if let Some(headers_end)=request.windows(4).position(|bytes|bytes==b"\r\n\r\n") {
                let headers=String::from_utf8_lossy(&request[..headers_end]);
                let length=headers.lines().find_map(|line|line.split_once(':').filter(|(name,_)|name.eq_ignore_ascii_case("content-length"))
                    .map(|(_,value)|value.trim().parse::<usize>().unwrap())).unwrap_or(0);
                if request.len()>=headers_end+4+length {return request;}
            }
        }
    }

    fn tts_call(base: &str) -> ResolvedCall {
        crate::adapters::test_support::call_with_endpoint(base, "tts-1", "openai.audio_speech", "/audio/speech",
            TaskRequest::SpeechSynthesis(TtsRequest { text: "Hello".into(), voice: None, format: Some("pcm".into()), extra: json!({}) }))
    }

    fn asr_config(base: &str) -> ResolvedTaskConfig {
        ResolvedTaskConfig {
            provider_id: "018f0000-0000-7000-8000-000000000001".into(), config_revision: 1,
            platform: "deepgram".into(), model: "nova-3".into(), task: ModelTask::SpeechRecognition,
            traits: vec![], input_constraints: None, protocol: DEEPGRAM_INCREMENTAL_ASR_PROTOCOL.into(),
            connection: ResolvedConnection { role: "default".into(), base_url: base.into(),
                auth: AuthMaterial { scheme: AuthScheme::TokenHeader, credentials: json!({"api_keys":["test-key"]}) }, extra: json!({}) },
            transport: ResolvedTaskTransport { realtime_endpoint: Some("/v1/listen".into()), ..Default::default() },
            provider_params: json!({"language":"zh", "punctuate":true}), context_limit: None, output_limit: None, bedrock_config: None,
        }
    }

    #[test]
    fn capabilities_do_not_confuse_sse_or_batch_upload_with_incremental_input() {
        for protocol in ["deepgram.listen", "stepfun.asr_sse", "openai.audio_transcriptions", "stepfun.audio_speech"] {
            assert_eq!(protocol_capabilities(protocol), SpeechProtocolCapabilities::default(), "{protocol}");
        }
        assert!(protocol_capabilities(DEEPGRAM_INCREMENTAL_ASR_PROTOCOL).incremental_asr);
        assert!(protocol_capabilities("openai.audio_speech").streaming_tts);
        let descriptor = crate::manifest::protocol_descriptor(DEEPGRAM_INCREMENTAL_ASR_PROTOCOL).unwrap();
        assert_eq!(descriptor.transport, nomifun_api_types::ProtocolTransportKind::Websocket);
        assert_eq!(descriptor.supported_tasks, [ModelTask::SpeechRecognition]);
        assert_eq!(descriptor.allowed_auth_schemes, ["token", "bearer"]);
    }

    #[tokio::test]
    async fn tts_yields_provider_pcm_before_response_eof_and_preserves_split_samples() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1", listener.local_addr().unwrap());
        let (release, held) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut wire, _) = listener.accept().await.unwrap();
            let request=read_request(&mut wire).await;
            let request = String::from_utf8_lossy(&request);
            assert!(request.contains("\"response_format\":\"pcm\""));
            assert!(!request.contains("stream_format"));
            wire.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/pcm\r\nTransfer-Encoding: chunked\r\n\r\n3\r\n\x01\x02\x03\r\n").await.unwrap();
            held.await.unwrap();
            wire.write_all(b"3\r\n\x04\x05\x06\r\n0\r\n\r\n").await.unwrap();
        });
        let mut response = open_pcm_response(&reqwest::Client::new(), &tts_call(&base)).await.unwrap();
        assert_eq!(response.sample_rate, 24_000);
        assert_eq!(tokio::time::timeout(Duration::from_secs(2), response.audio.next()).await.unwrap().unwrap().unwrap(), [0x0201]);
        assert!(!server.is_finished(), "first provider audio must precede full synthesis completion");
        release.send(()).unwrap();
        assert_eq!(response.audio.next().await.unwrap().unwrap(), [0x0403, 0x0605]);
        assert!(response.audio.next().await.is_none());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn tts_rejects_incomplete_pcm_and_json_even_with_success_status() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::{method, path}};
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/v1/audio/speech"))
            .respond_with(ResponseTemplate::new(200).insert_header("content-type", "audio/pcm").set_body_bytes(vec![1, 2, 3]))
            .mount(&server).await;
        let mut response = open_pcm_response(&reqwest::Client::new(), &tts_call(&format!("{}/v1",server.uri()))).await.unwrap();
        assert_eq!(response.audio.next().await.unwrap().unwrap(), [0x0201]);
        assert_eq!(response.audio.next().await.unwrap().unwrap_err().kind, InvokeErrorKind::ParseError);
        server.reset().await;
        Mock::given(method("POST")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"error":"not audio"}))).mount(&server).await;
        let error = open_pcm_response(&reqwest::Client::new(), &tts_call(&format!("{}/v1",server.uri()))).await.err().unwrap();
        assert_eq!(error.kind, InvokeErrorKind::ParseError);
    }

    #[tokio::test]
    async fn dropping_tts_stream_closes_the_exact_incomplete_provider_response() {
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base=format!("http://{}/v1",listener.local_addr().unwrap());
        let server=tokio::spawn(async move {
            let (mut wire,_)=listener.accept().await.unwrap();
            read_request(&mut wire).await;
            wire.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/pcm\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n\x01\x02\r\n").await.unwrap();
            let mut tail=[0;1];
            let closed=tokio::time::timeout(Duration::from_secs(3),wire.read(&mut tail)).await.unwrap();
            assert!(matches!(closed,Ok(0)|Err(_)),"dropping the request must close its incomplete response");
        });
        let mut stream=open_pcm_response(&reqwest::Client::new(),&tts_call(&base)).await.unwrap();
        assert_eq!(stream.audio.next().await.unwrap().unwrap(),[0x0201]);
        drop(stream);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn asr_handshake_preserves_the_explicit_token_or_bearer_scheme() {
        for (scheme, expected) in [(AuthScheme::TokenHeader, "Token test-key"), (AuthScheme::Bearer, "Bearer test-key")] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (wire, _) = listener.accept().await.unwrap();
                accept_hdr_async(wire, move |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.headers()["authorization"], expected);
                    Ok(response)
                }).await.unwrap()
            });
            let mut config = asr_config(&base);
            config.connection.auth.scheme = scheme;
            let session = create_transcription_session(config, 16_000, None).await.unwrap();
            let _wire = server.await.unwrap();
            session.cancel_and_wait().await;
        }
    }

    #[tokio::test]
    async fn asr_sends_pcm_before_endpointing_and_only_admits_final_transcript_spans() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (wire, _) = listener.accept().await.unwrap();
            let mut wire = accept_hdr_async(wire, |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                assert_eq!(request.headers()["authorization"], "Token test-key");
                let query = request.uri().query().unwrap();
                assert!(query.contains("sample_rate=16000"));
                assert!(query.contains("encoding=linear16"));
                assert!(query.contains("model=nova-3"));
                Ok(response)
            }).await.unwrap();
            let audio = wire.next().await.unwrap().unwrap();
            assert!(matches!(audio, Message::Binary(ref bytes) if bytes.len()==1920));
            for value in [json!({"type":"Results","start":0.0,"is_final":false,"channel":{"alternatives":[{"transcript":"可撤回草稿"}]}}),
                json!({"type":"Results","start":0.0,"is_final":true,"channel":{"alternatives":[{"transcript":"天气"}]}})] {
                wire.send(Message::Text(value.to_string().into())).await.unwrap();
            }
            let close = wire.next().await.unwrap().unwrap().into_text().unwrap();
            assert_eq!(serde_json::from_str::<Value>(&close).unwrap()["type"], "CloseStream");
            wire.send(Message::Text(json!({"type":"Results","start":0.5,"is_final":true,"channel":{"alternatives":[{"transcript":"晴朗"}]}}).to_string().into())).await.unwrap();
            wire.send(Message::Text(json!({"type":"Metadata","duration":1.0}).to_string().into())).await.unwrap();
            wire.close(None).await.unwrap();
        });
        let mut session = create_transcription_session(asr_config(&base), 16_000, None).await.unwrap();
        session.try_send(vec![0; 960]).unwrap();
        assert_eq!(session.recv().await.unwrap().unwrap(), TranscriptEvent::Interim("可撤回草稿".into()));
        assert_eq!(session.recv().await.unwrap().unwrap(), TranscriptEvent::FinalSegment("天气".into()));
        assert!(!server.is_finished(), "provider transcribes before physical endpointing");
        assert_eq!(session.finish().await.unwrap(), "天气 晴朗");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn asr_abrupt_close_does_not_certify_finalization() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (wire, _) = listener.accept().await.unwrap();
            let mut wire = tokio_tungstenite::accept_async(wire).await.unwrap();
            assert!(matches!(wire.next().await.unwrap().unwrap(), Message::Text(_)));
            wire.close(None).await.unwrap();
        });
        let session = create_transcription_session(asr_config(&base), 16_000, None).await.unwrap();
        assert_eq!(session.finish().await.unwrap_err().kind, InvokeErrorKind::Network);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn asr_queue_and_frame_duration_are_bounded_and_cancel_joins_the_worker() {
        let (input, _input_rx) = mpsc::channel(1);
        let (_events_tx, events) = mpsc::channel(1);
        let (dropped, was_dropped) = oneshot::channel();
        let entered=std::sync::Arc::new(tokio::sync::Notify::new());
        let ready=entered.clone();
        let worker = tokio::spawn(async move {
            struct Witness(Option<oneshot::Sender<()>>);
            impl Drop for Witness { fn drop(&mut self) { let _ = self.0.take().unwrap().send(()); } }
            let _witness = Witness(Some(dropped));
            ready.notify_one();
            std::future::pending::<()>().await;
        });
        entered.notified().await;
        let session = TranscriptionSession { input: Some(input), events, worker: Some(worker), sample_rate: 16_000 };
        assert!(session.try_send(vec![0;1921]).is_err());
        session.try_send(vec![0;960]).unwrap();
        assert!(session.try_send(vec![0;960]).is_err());
        session.cancel_and_wait().await;
        was_dropped.await.unwrap();
    }
}
