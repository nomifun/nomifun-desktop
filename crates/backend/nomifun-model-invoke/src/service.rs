//! [`ModelInvokeService`] — the invoke layer's entry point: catalog
//! repositories + credential decryption key + shared HTTP client + the
//! protocol adapter registry. This module carries the constructor and the
//! invoke / poll / probe flows; the catalog resolution pipeline they
//! all share lives in [`crate::resolve`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::Engine as _;
use nomifun_api_types::ModelTask;
use serde_json::json;

use crate::adapter::AdapterRegistry;
use crate::adapters::default_realtime_adapters;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::realtime::{
    RealtimeAdapterRegistry, RealtimeServerEvent, RealtimeSession, RealtimeSessionConfig,
};
use crate::types::{
    AsrRequest, EmbedRequest, ImageEditRequest, ImageGenRequest, InputAsset, JobHandle, ModelRef,
    RerankRequest, TaskOutcome, TaskRequest, TtsRequest, VideoGenRequest,
};

/// Ceiling on one modality probe (resolution + submit).
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

/// A valid 512x512 white PNG used by ImageEdit probes. A 1x1 image reaches the
/// wire but is below documented provider dimension limits, producing a false
/// unhealthy result even when endpoint, credentials and model are correct.
const PROBE_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAgAAAAIAAQMAAADOtka5AAAAAXNSR0IArs4c6QAAAARnQU1BAACxjwv8YQUAAAAGUExURf///////1V89WwAAAAJcEhZcwAADsMAAA7DAcdvqGQAAAA2SURBVHja7cEBAQAAAIIg/69uSEABAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAHwbggAAAWN1UKQAAAAASUVORK5CYII=";

fn probe_png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(PROBE_PNG_BASE64)
        .expect("embedded health-probe PNG is valid Base64")
}

/// A short, valid 16 kHz mono PCM16 WAV containing silence. Empty byte arrays
/// are rejected before model validation by many ASR providers, which would
/// make every correctly configured transcription model look unhealthy.
fn probe_wav() -> Vec<u8> {
    const SAMPLE_RATE: u32 = 16_000;
    const SAMPLE_COUNT: u32 = 1_600; // 100 ms
    const DATA_LEN: u32 = SAMPLE_COUNT * 2;
    let mut wav = Vec::with_capacity((44 + DATA_LEN) as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + DATA_LEN).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // mono
    wav.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    wav.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&DATA_LEN.to_le_bytes());
    wav.resize((44 + DATA_LEN) as usize, 0);
    wav
}

fn validate_job_resume(
    expected_protocol: &str,
    expected_config_revision: i64,
    job: &JobHandle,
) -> Result<(), InvokeError> {
    if job.adapter_id != expected_protocol {
        return Err(InvokeError::config(format!(
            "pending job protocol {:?} no longer matches the configured capability protocol {:?}",
            job.adapter_id, expected_protocol
        )));
    }
    if job.config_revision != expected_config_revision {
        return Err(InvokeError::config(format!(
            "pending job configuration revision {} no longer matches provider revision {}; submit a new job",
            job.config_revision, expected_config_revision
        )));
    }
    Ok(())
}

fn bind_pending_job(
    expected_protocol: &str,
    config_revision: i64,
    mut outcome: TaskOutcome,
) -> Result<TaskOutcome, InvokeError> {
    if let TaskOutcome::Pending(job) = &mut outcome {
        if job.adapter_id != expected_protocol {
            return Err(InvokeError::config(format!(
                "protocol adapter {:?} returned a job owned by {:?}",
                expected_protocol, job.adapter_id
            )));
        }
        job.config_revision = config_revision;
    }
    Ok(outcome)
}

/// Outcome of a health probe. Upstream failures never surface as `Err` — they
/// fold into `healthy = false` + `message`; `Err` is reserved for calls the
/// probe path does not serve at all (chat rides the agent engine).
#[derive(Debug, Clone)]
pub struct ProbeReport {
    pub healthy: bool,
    /// Wall time of the probe attempt (resolution is a few local DB reads;
    /// this effectively measures the submit round-trip).
    pub latency_ms: u64,
    /// Failure detail when `healthy == false`.
    pub message: Option<String>,
}

/// The unified multimodal model invocation service.
pub struct ModelInvokeService {
    pub(crate) provider_repo: Arc<dyn nomifun_db::IProviderRepository>,
    pub(crate) provider_model_repo: Arc<dyn nomifun_db::IProviderModelRepository>,
    pub(crate) provider_model_capability_repo:
        Arc<dyn nomifun_db::IProviderModelCapabilityRepository>,
    pub(crate) provider_connection_repo: Arc<dyn nomifun_db::IProviderConnectionRepository>,
    /// AES-256-GCM key used to decrypt stored provider/connection credentials.
    pub(crate) encryption_key: [u8; 32],
    /// Shared client for all adapter calls.
    pub(crate) http: reqwest::Client,
    pub(crate) registry: AdapterRegistry,
    /// Persistent WebSocket/session protocols live in a separate registry so
    /// they cannot be selected by the one-shot HTTP dispatcher.
    pub(crate) realtime_registry: RealtimeAdapterRegistry,
}

impl ModelInvokeService {
    pub fn new(
        provider_repo: Arc<dyn nomifun_db::IProviderRepository>,
        provider_model_repo: Arc<dyn nomifun_db::IProviderModelRepository>,
        provider_model_capability_repo: Arc<dyn nomifun_db::IProviderModelCapabilityRepository>,
        provider_connection_repo: Arc<dyn nomifun_db::IProviderConnectionRepository>,
        encryption_key: [u8; 32],
        http: reqwest::Client,
        registry: AdapterRegistry,
    ) -> Self {
        Self {
            provider_repo,
            provider_model_repo,
            provider_model_capability_repo,
            provider_connection_repo,
            encryption_key,
            http,
            registry,
            realtime_registry: RealtimeAdapterRegistry::new(default_realtime_adapters()),
        }
    }

    /// Replace the realtime registry, primarily for deterministic session
    /// adapter contract tests and embedders adding their own provider plugin.
    pub fn with_realtime_registry(mut self, registry: RealtimeAdapterRegistry) -> Self {
        self.realtime_registry = registry;
        self
    }

    /// The provider repository this service resolves against. Shared with
    /// callers (e.g. the creation engine) that pre-check provider existence
    /// before enqueueing work, so they don't need a second repo handle.
    pub fn provider_repo(&self) -> &Arc<dyn nomifun_db::IProviderRepository> {
        &self.provider_repo
    }

    /// Capability repository used for task-scoped health observations. It is
    /// exposed through the resolver service so callers never need a parallel
    /// model-level configuration dependency.
    pub fn provider_model_capability_repo(
        &self,
    ) -> &Arc<dyn nomifun_db::IProviderModelCapabilityRepository> {
        &self.provider_model_capability_repo
    }

    /// Execute one task invocation: exact task-capability resolution followed
    /// by the selected protocol adapter. A [`TaskOutcome::Pending`] hands
    /// back a [`JobHandle`] the caller later feeds to [`Self::poll`].
    pub async fn invoke(&self, m: &ModelRef, req: TaskRequest) -> Result<TaskOutcome, InvokeError> {
        let task = req.task();
        let (call, adapter) = self.resolve(m, task, req).await?;
        let redactor = call.connection.auth.secret_redactor();
        let outcome = adapter
            .submit(&self.http, &call)
            .await
            .map_err(|error| error.redacted(&redactor))?;
        bind_pending_job(&call.protocol, call.config_revision, outcome)
    }

    /// Open a live, bidirectional model session through the dedicated
    /// realtime resolver and adapter registry.
    ///
    /// This entry point deliberately cannot accept a [`TaskRequest`]: catalog
    /// rows must explicitly declare [`ModelTask::RealtimeConversation`], and a
    /// one-shot HTTP adapter can never be selected for the socket transport.
    pub async fn open_realtime(
        &self,
        m: &ModelRef,
        config: RealtimeSessionConfig,
    ) -> Result<RealtimeSession, InvokeError> {
        let (call, adapter) = self.resolve_realtime(m).await?;
        let redactor = call.connection.auth.secret_redactor();
        adapter
            .open(&call, config)
            .await
            .map_err(|error| error.redacted(&redactor))
    }

    /// Poll a pending job after resolving the same exact task capability used
    /// by ordinary invocations. The persisted job pins both its protocol and
    /// the provider's monotonic invocation-graph revision. Any endpoint,
    /// credential, connection, model or capability change fails closed rather
    /// than resuming a remote job with different transport authority.
    pub async fn poll(
        &self,
        m: &ModelRef,
        req: TaskRequest,
        job: &JobHandle,
    ) -> Result<TaskOutcome, InvokeError> {
        let task = req.task();
        let (call, adapter) = self.resolve(m, task, req).await?;
        validate_job_resume(&call.protocol, call.config_revision, job)?;
        let redactor = call.connection.auth.secret_redactor();
        let outcome = adapter
            .poll(&self.http, &call, job)
            .await
            .map_err(|error| error.redacted(&redactor))?;
        bind_pending_job(&call.protocol, call.config_revision, outcome)
    }

    /// Health-probe `(model, task)` with the smallest valid request:
    ///
    /// - resolution requires the same explicit task capability as a real call
    ///   (disabled provider/model rows still refuse and fold into unhealthy);
    /// - `Done` OR `Pending` → healthy (an async accept proves the endpoint;
    ///   the probe never polls and never downloads content);
    /// - every upstream error, including 400/422 `InvalidParams`, remains
    ///   unhealthy. A generic probe cannot prove that the error is limited to
    ///   its placeholder input rather than a wrong/retired model or protocol;
    /// - the whole attempt is capped at 60 s.
    ///
    /// Agent and persistent-session tasks have no one-shot request shape and
    /// are rejected as unsupported; Chat health runs through the agent engine
    /// and realtime health uses [`Self::probe_realtime`].
    pub async fn probe(&self, m: &ModelRef, task: ModelTask) -> Result<ProbeReport, InvokeError> {
        let started = Instant::now();
        let attempt = async {
            // Two-phase build: resolve with a params-free placeholder, then
            // rebuild the minimal request from the resolved model_params so
            // catalog defaults (image size/quality, TTS voice, …) ride along —
            // protocol-specific request defaults ride along with the typed probe.
            let placeholder = probe_request(task, &json!({})).ok_or_else(|| {
                InvokeError::new(
                    InvokeErrorKind::UnsupportedTask,
                    format!("task {task:?} has no one-shot probe request"),
                )
            })?;
            let (mut call, adapter) = self.resolve(m, task, placeholder).await?;
            call.request = probe_request(task, &call.model_params)
                .expect("a task with an initial one-shot probe remains one-shot");
            let redactor = call.connection.auth.secret_redactor();
            adapter
                .submit(&self.http, &call)
                .await
                .map_err(|error| error.redacted(&redactor))
        };
        let outcome = tokio::time::timeout(PROBE_TIMEOUT, attempt).await;
        let latency_ms = started.elapsed().as_millis() as u64;
        let report = match outcome {
            // Sync success, or async accepted — either proves the model
            // answers. Pending is NOT followed up: no poll, no download.
            Ok(Ok(TaskOutcome::Done(_) | TaskOutcome::Pending(_))) => {
                ProbeReport { healthy: true, latency_ms, message: None }
            }
            Ok(Err(e)) => ProbeReport { healthy: false, latency_ms, message: Some(e.to_string()) },
            Err(_) => ProbeReport {
                healthy: false,
                latency_ms,
                message: Some(format!("modality probe timeout ({}s)", PROBE_TIMEOUT.as_secs())),
            },
        };
        Ok(report)
    }

    /// Probe a realtime model by completing its WebSocket handshake, waiting
    /// for the provider's `session.created` acknowledgement, and then closing
    /// the process-local session cleanly.
    ///
    /// As with [`Self::probe`], catalog, adapter and upstream failures are
    /// folded into an unhealthy report. The dedicated method keeps persistent
    /// sessions out of the one-shot probe/request union.
    pub async fn probe_realtime(&self, m: &ModelRef) -> Result<ProbeReport, InvokeError> {
        let started = Instant::now();
        let attempt = async {
            let (call, adapter) = self.resolve_realtime(m).await?;
            let redactor = call.connection.auth.secret_redactor();
            let mut session = adapter
                .open(&call, RealtimeSessionConfig::default())
                .await
                .map_err(|error| error.redacted(&redactor))?;

            loop {
                match session.recv().await {
                    Some(RealtimeServerEvent::SessionCreated { .. }) => {
                        // A close failure after the provider acknowledged the
                        // session does not invalidate the connectivity probe;
                        // `RealtimeSession::close` is itself bounded and will
                        // abort a stuck worker before returning.
                        let _ = session.close().await;
                        return Ok::<(), InvokeError>(());
                    }
                    Some(RealtimeServerEvent::ProviderError { message, .. }) => {
                        return Err(InvokeError::new(
                            InvokeErrorKind::ProviderError,
                            format!(
                                "realtime provider error: {}",
                                redactor.redact(&message)
                            ),
                        ));
                    }
                    Some(RealtimeServerEvent::TransportError { message }) => {
                        return Err(InvokeError::new(
                            InvokeErrorKind::Network,
                            redactor.redact(&message),
                        ));
                    }
                    Some(RealtimeServerEvent::Closed { code, reason }) => {
                        return Err(InvokeError::new(
                            InvokeErrorKind::Network,
                            format!(
                                "realtime session closed before creation (code {code:?}): {}",
                                redactor.redact(&reason)
                            ),
                        ));
                    }
                    Some(_) => {
                        // Providers may emit informational events around
                        // session creation; only the acknowledgement proves
                        // the requested model and credentials were accepted.
                    }
                    None => {
                        return Err(InvokeError::new(
                            InvokeErrorKind::Network,
                            "realtime session ended before creation",
                        ));
                    }
                }
            }
        };

        let outcome = tokio::time::timeout(PROBE_TIMEOUT, attempt).await;
        let latency_ms = started.elapsed().as_millis() as u64;
        let report = match outcome {
            Ok(Ok(())) => ProbeReport { healthy: true, latency_ms, message: None },
            Ok(Err(error)) => {
                ProbeReport { healthy: false, latency_ms, message: Some(error.to_string()) }
            }
            Err(_) => ProbeReport {
                healthy: false,
                latency_ms,
                message: Some(format!(
                    "realtime probe timeout ({}s)",
                    PROBE_TIMEOUT.as_secs()
                )),
            },
        };
        Ok(report)
    }
}

/// The smallest valid [`TaskRequest`] for a probe, overlaying known catalog
/// params (image size/quality + steps/cfg_scale/text_mode passthrough, TTS
/// voice) without introducing a second transport configuration source.
fn probe_request(
    task: ModelTask,
    params: &serde_json::Value,
) -> Option<TaskRequest> {
    match task {
        ModelTask::ImageGeneration => Some(TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: "health check".into(),
            count: 1,
            size: params.get("size").and_then(|v| v.as_str()).map(str::to_string),
            quality: params.get("quality").and_then(|v| v.as_str()).map(str::to_string),
            extra: {
                // Params only some providers require; adapters that understand
                // them (ark images steps/cfg_scale/text_mode) read `extra`.
                let mut extra = serde_json::Map::new();
                for key in ["steps", "cfg_scale", "text_mode"] {
                    if let Some(v) = params.get(key) {
                        extra.insert(key.to_string(), v.clone());
                    }
                }
                serde_json::Value::Object(extra)
            },
        })),
        ModelTask::ImageEdit => Some(TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "health check".into(),
            count: 1,
            size: None,
            // A real (minimal) PNG: openai.images rejects an input-less edit
            // locally, and a probe that never reaches the wire is vacuous.
            // The stub is not meaningful content. Any upstream rejection still
            // remains unhealthy because it may also indicate a retired model
            // or incompatible protocol.
            inputs: vec![InputAsset {
                id: None,
                role: "reference".into(),
                bytes: probe_png(),
                mime: "image/png".into(),
            }],
            extra: json!({}),
        })),
        ModelTask::VideoGeneration => Some(TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "health check".into(),
            seconds: None,
            size: None,
            inputs: vec![],
            extra: json!({}),
        })),
        ModelTask::SpeechSynthesis => Some(TaskRequest::SpeechSynthesis(TtsRequest {
            text: "hi".into(),
            // Use a model-level user override first. For provider/model pairs
            // whose official API publishes a stable system voice, use that
            // exact provider-scoped value so a healthy TTS model does not fail
            // its probe merely because the real generation UI supplies voice
            // per request. Unknown providers/models remain None and therefore
            // fail closed rather than guessing an OpenAI voice.
            voice: params
                .get("voice")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            format: None,
            extra: json!({}),
        })),
        ModelTask::SpeechRecognition => Some(TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset {
                id: None,
                role: "audio".into(),
                bytes: probe_wav(),
                mime: "audio/wav".into(),
            },
            language: None,
            prompt: None,
            extra: json!({}),
        })),
        ModelTask::Embedding => {
            Some(TaskRequest::Embedding(EmbedRequest { inputs: vec!["health check".into()], extra: json!({}) }))
        }
        ModelTask::Rerank => Some(TaskRequest::Rerank(RerankRequest {
            query: "health check".into(),
            documents: vec!["health check".into()],
            top_n: Some(1),
            extra: json!({}),
        })),
        ModelTask::Chat | ModelTask::RealtimeConversation => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_assets_have_expected_container_signatures() {
        assert_eq!(&probe_png()[..8], b"\x89PNG\r\n\x1a\n");
        let wav = probe_wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    fn job(protocol: &str, config_revision: i64) -> JobHandle {
        JobHandle {
            adapter_id: protocol.to_owned(),
            config_revision,
            remote_id: "remote-job".into(),
            poll_state: serde_json::Value::Null,
        }
    }

    #[test]
    fn async_resume_rejects_a_protocol_changed_after_submit() {
        let error = validate_job_resume("xai.video_jobs", 9, &job("openai.videos", 9))
            .unwrap_err();
        assert!(error.message.contains("protocol"));
    }

    #[test]
    fn async_resume_rejects_same_protocol_after_endpoint_or_auth_revision_changes() {
        let error = validate_job_resume("openai.videos", 10, &job("openai.videos", 9))
            .unwrap_err();
        assert!(error.message.contains("configuration revision"));
        validate_job_resume("openai.videos", 10, &job("openai.videos", 10)).unwrap();
    }
}
