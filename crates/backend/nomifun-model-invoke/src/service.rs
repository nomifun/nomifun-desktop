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

use crate::adapter::{AdapterRegistry, ProtocolAdapter};
use crate::adapters::default_realtime_adapters;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::realtime::{
    RealtimeAdapterRegistry, RealtimeServerEvent, RealtimeSession, RealtimeSessionConfig,
};
use crate::types::{
    AsrRequest, EmbedRequest, ImageEditRequest, ImageGenRequest, InputAsset, JobHandle, ModelRef,
    RerankRequest, TaskOutcome, TaskRequest, TtsRequest, VideoGenRequest,
};
use crate::ResolvedCall;

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

/// A short, valid 16 kHz mono PCM16 WAV. Empty byte arrays are rejected before
/// model validation by many ASR providers, which would make every correctly
/// configured transcription model look unhealthy.
///
/// The samples are a quiet tone rather than digital silence, so the payload is
/// representative audio instead of a buffer some providers special-case. Note
/// this does not make the probe verify transcription quality: `probe` classifies
/// on the outcome variant and never inspects the transcript, by design — it
/// answers "did this endpoint/model/credential combination answer", and an
/// empty transcript is a valid answer.
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
    // 440 Hz at roughly -20 dBFS.
    for index in 0..SAMPLE_COUNT {
        let phase = 2.0 * std::f32::consts::PI * 440.0 * (index as f32) / (SAMPLE_RATE as f32);
        let sample = (phase.sin() * 3_276.0) as i16;
        wav.extend_from_slice(&sample.to_le_bytes());
    }
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

/// Opaque state captured from the exact catalog resolution used for a submit.
///
/// Its fields intentionally remain crate-private: the resolved call includes
/// decrypted authentication material. Callers can only hand the context back
/// to this service for stable polling and artifact materialization.
pub struct InvocationContext {
    pub(crate) call: ResolvedCall,
    pub(crate) adapter: Arc<dyn ProtocolAdapter>,
    pub(crate) artifact_origin: reqwest::Url,
}

impl InvocationContext {
    fn from_resolved(
        call: ResolvedCall,
        adapter: Arc<dyn ProtocolAdapter>,
    ) -> Result<Self, InvokeError> {
        call.connection.auth.validate()?;
        let artifact_origin = validated_artifact_origin(&call)?;
        Ok(Self {
            call,
            adapter,
            artifact_origin,
        })
    }
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

    /// Resolve and validate a catalog model locally without making an upstream
    /// request.  This is the capability-discovery counterpart to [`Self::invoke`]:
    /// it shares the exact provider/model/task/adapter/connection resolver, then
    /// validates the resulting URL and auth material while keeping decrypted
    /// credentials inside the service boundary.
    pub async fn validate(&self, m: &ModelRef, task: ModelTask) -> Result<(), InvokeError> {
        self.resolve_validated_call(m, task).await.map(|_| ())
    }

    /// Internal form of [`Self::validate`] retained for trusted invoke-layer
    /// consumers such as artifact materialization.  The call (and therefore
    /// decrypted auth) never crosses the crate's public API.
    pub(crate) async fn resolve_validated_call(
        &self,
        m: &ModelRef,
        task: ModelTask,
    ) -> Result<ResolvedCall, InvokeError> {
        let request = probe_request(task, &json!({})).ok_or_else(|| {
            InvokeError::new(
                InvokeErrorKind::UnsupportedTask,
                format!("task {task:?} has no one-shot validation request"),
            )
        })?;
        let (call, _adapter) = self.resolve(m, task, request).await?;
        validated_artifact_origin(&call)?;
        call.connection.auth.validate()?;
        Ok(call)
    }

    /// Execute one task invocation: exact task-capability resolution followed
    /// by the selected protocol adapter. A [`TaskOutcome::Pending`] hands
    /// back a [`JobHandle`] the caller later feeds to [`Self::poll`].
    pub async fn invoke(&self, m: &ModelRef, req: TaskRequest) -> Result<TaskOutcome, InvokeError> {
        self.invoke_with_context(m, req)
            .await
            .map(|(outcome, _context)| outcome)
    }

    /// Submit and return opaque state from that exact resolution. This avoids
    /// re-reading a mutable catalog after an accepted/billable generation when
    /// the caller later polls or materializes its assets.
    pub async fn invoke_with_context(
        &self,
        m: &ModelRef,
        req: TaskRequest,
    ) -> Result<(TaskOutcome, InvocationContext), InvokeError> {
        let task = req.task();
        let (call, adapter) = self.resolve(m, task, req).await?;
        let context = InvocationContext::from_resolved(call, adapter)?;
        let redactor = context.call.connection.auth.secret_redactor();
        let outcome = context
            .adapter
            .submit(&self.http, &context.call)
            .await
            .map_err(|error| error.redacted(&redactor))?;
        let outcome = bind_pending_job(
            &context.call.protocol,
            context.call.config_revision,
            outcome,
        )?;
        Ok((outcome, context))
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

    /// Poll with the immutable resolved call and adapter captured by
    /// [`Self::invoke_with_context`]. Catalog edits made after submit cannot
    /// invalidate or reroute the already-accepted job.
    pub async fn poll_with_context(
        &self,
        context: &InvocationContext,
        req: TaskRequest,
        job: &JobHandle,
    ) -> Result<TaskOutcome, InvokeError> {
        let task = req.task();
        if task != context.call.task {
            return Err(InvokeError::new(
                InvokeErrorKind::InvalidParams,
                format!(
                    "poll request task {task:?} does not match submitted task {:?}",
                    context.call.task
                ),
            ));
        }
        if job.adapter_id != context.adapter.id() {
            return Err(InvokeError::new(
                InvokeErrorKind::InvalidParams,
                format!(
                    "job adapter {:?} does not match submitted adapter {:?}",
                    job.adapter_id,
                    context.adapter.id()
                ),
            ));
        }
        let mut call = context.call.clone();
        call.request = req;
        validate_job_resume(&call.protocol, call.config_revision, job)?;
        let redactor = call.connection.auth.secret_redactor();
        let outcome = context
            .adapter
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
            call.request = probe_request_for_protocol(task, &call.model_params, &call.protocol)
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

fn validated_artifact_origin(call: &ResolvedCall) -> Result<reqwest::Url, InvokeError> {
    let endpoint = call.endpoint_url()?;
    let mut url = reqwest::Url::parse(&endpoint).map_err(|error| {
        InvokeError::config(format!("resolved endpoint is not a valid URL: {error}"))
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(InvokeError::config(format!(
            "resolved endpoint uses unsupported scheme {:?}",
            url.scheme()
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(InvokeError::config(
            "resolved endpoint must not contain embedded credentials",
        ));
    }
    // Retain only the origin needed for same-origin/private-network policy;
    // endpoint paths, query parameters and fragments never cross this opaque
    // context boundary or appear in materialization errors.
    url.set_path("/");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

/// The smallest valid [`TaskRequest`] for a probe, overlaying known catalog
/// params (image size/quality + steps/cfg_scale/text_mode passthrough, TTS
/// voice) without introducing a second transport configuration source.
fn probe_request(
    task: ModelTask,
    params: &serde_json::Value,
) -> Option<TaskRequest> {
    probe_request_for_protocol(task, params, "")
}

/// A protocol whose official API publishes stable system voices but which
/// rejects a request carrying none. The catalog voice always wins; this only
/// keeps a correctly configured model from failing its probe locally when the
/// user has not set a default voice.
///
/// Keyed by protocol rather than platform, matching the UI's own suggestion
/// table (`ttsVoiceOptions.ts`): one provider may host models served by
/// different adapters.
///
/// Scope, deliberately: this substitution exists ONLY in the probe. It answers
/// "is this endpoint/model/credential combination reachable", which is what a
/// health check measures. It does NOT prove the generation path is configured:
/// with an empty `provider_params.voice` AND no companion/global voice, real
/// synthesis still fails locally in `build_tts_body`. The durable fix for that
/// is the default-voice field in model management, which writes
/// `provider_params.voice` — a value the adapter merges into every request
/// (see `configured_catalog_voice_is_used_when_the_request_carries_none` in
/// `adapters::stepfun`), so a configured model is consistent in both paths.
fn probe_fallback_voice(protocol: &str) -> Option<&'static str> {
    match protocol {
        // An official StepFun system voice; the metered API and Step Plan
        // share the same ids.
        "stepfun.audio_speech" => Some("cixingnansheng"),
        _ => None,
    }
}

fn probe_request_for_protocol(
    task: ModelTask,
    params: &serde_json::Value,
    protocol: &str,
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
            //
            // The fallback applies only when NO voice key is configured. A key
            // that is present but blank or non-string is broken configuration:
            // substituting there would make the probe pass while real synthesis
            // keeps failing on the same stored value.
            voice: match params.get("voice") {
                Some(configured) => configured
                    .as_str()
                    .map(str::trim)
                    .filter(|voice| !voice.is_empty())
                    .map(str::to_string),
                None => probe_fallback_voice(protocol).map(str::to_string),
            },
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
    use std::sync::Arc;

    use nomifun_api_types::ModelTask;
    use nomifun_common::encrypt_string;
    use nomifun_db::{
        CreateProviderParams, IProviderConnectionRepository, IProviderModelRepository,
        IProviderRepository, NewProviderModel, NewProviderModelCapability,
        SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
        SqliteProviderModelRepository, SqliteProviderRepository, SqlitePool,
        UpdateProviderParams, UpsertProviderConnectionParams, init_database_memory,
    };
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::{
        AdapterRegistry, ProducedData, ProtocolEndpointPurpose, TaskResult, default_adapters,
        preset_protocol_recommendation, protocol_task_descriptor,
    };

    const TEST_KEY: [u8; 32] = [0x42; 32];

    struct CapabilitySeed {
        task: String,
        protocol: &'static str,
        connection_role: &'static str,
        endpoint: Option<String>,
        poll_endpoint: Option<String>,
        content_endpoint: Option<String>,
        realtime_endpoint: Option<String>,
    }

    fn capability_seed(platform: &str, task: ModelTask) -> CapabilitySeed {
        let route = preset_protocol_recommendation(platform, task)
            .unwrap_or_else(|| panic!("test platform {platform:?} has no route for {task:?}"));
        let descriptor = protocol_task_descriptor(route.protocol, task)
            .unwrap_or_else(|| panic!("missing descriptor for {} {task:?}", route.protocol));
        let endpoint = |purpose| {
            descriptor
                .endpoints
                .iter()
                .find(|endpoint| endpoint.purpose == purpose)
                .map(|endpoint| endpoint.default_value.clone())
        };
        CapabilitySeed {
            task: serde_json::to_value(task)
                .expect("task serializes")
                .as_str()
                .expect("task is a string")
                .to_owned(),
            protocol: route.protocol,
            connection_role: route.connection_role.unwrap_or("default"),
            endpoint: endpoint(ProtocolEndpointPurpose::Submit),
            poll_endpoint: endpoint(ProtocolEndpointPurpose::Poll),
            content_endpoint: endpoint(ProtocolEndpointPurpose::Content),
            realtime_endpoint: endpoint(ProtocolEndpointPurpose::Session),
        }
    }

    fn capability_input<'a>(
        seed: &'a CapabilitySeed,
        provider_params: &'a str,
    ) -> NewProviderModelCapability<'a> {
        NewProviderModelCapability {
            task: &seed.task,
            traits: "[]",
            protocol: seed.protocol,
            connection_role: seed.connection_role,
            base_url_override: None,
            endpoint: seed.endpoint.as_deref(),
            poll_endpoint: seed.poll_endpoint.as_deref(),
            content_endpoint: seed.content_endpoint.as_deref(),
            realtime_endpoint: seed.realtime_endpoint.as_deref(),
            allow_cross_origin_credentials: false,
            provider_params,
            context_limit: None,
        }
    }

    async fn setup() -> (ModelInvokeService, SqlitePool) {
        let database = init_database_memory().await.expect("database");
        let pool = database.pool().clone();
        let service = ModelInvokeService::new(
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
            Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
            TEST_KEY,
            reqwest::Client::new(),
            AdapterRegistry::new(default_adapters()),
        );
        (service, pool)
    }

    async fn provider_revision(pool: &SqlitePool, provider_id: &str) -> i64 {
        SqliteProviderRepository::new(pool.clone())
            .find_by_id(provider_id)
            .await
            .expect("provider lookup")
            .expect("provider exists")
            .config_revision
    }

    #[test]
    fn probe_assets_have_expected_container_signatures() {
        assert_eq!(&probe_png()[..8], b"\x89PNG\r\n\x1a\n");
        let wav = probe_wav();
        assert_eq!(&wav[..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");

        // The declared chunk sizes must match the bytes actually present: a
        // truncated WAV (header promising samples it never wrote) is rejected
        // by many decoders, which is exactly the false-unhealthy probe this
        // asset exists to avoid.
        let riff_size = u32::from_le_bytes(wav[4..8].try_into().unwrap()) as usize;
        let data_len = u32::from_le_bytes(wav[40..44].try_into().unwrap()) as usize;
        assert_eq!(riff_size, wav.len() - 8, "RIFF size must cover everything after it");
        assert_eq!(data_len, wav.len() - 44, "data chunk size must match the payload");

        // And the payload must carry a signal. Digital silence transcribes to
        // nothing on a healthy model, so it cannot tell working from broken.
        assert!(
            wav[44..].iter().any(|byte| *byte != 0),
            "probe audio must not be digital silence"
        );
    }

    fn job(protocol: &str, config_revision: i64) -> JobHandle {
        JobHandle {
            adapter_id: protocol.to_owned(),
            config_revision,
            remote_id: "remote-job".into(),
            poll_state: serde_json::Value::Null,
        }
    }

    /// Seed an enabled openai-platform provider whose base_url is the mock
    /// server (key decrypts to `sk-test` → `Authorization: Bearer sk-test`).
    async fn seed_provider(pool: &SqlitePool, base_url: &str) -> String {
        let base_url = format!("{}/v1", base_url.trim_end_matches('/'));
        seed_provider_on(pool, "openai", &base_url).await
    }

    /// Platform-aware provider seeder (same key material as [`seed_provider`]).
    async fn seed_provider_on(pool: &SqlitePool, platform: &str, base_url: &str) -> String {
        let repo = SqliteProviderRepository::new(pool.clone());
        let encrypted = encrypt_string(r#"{"api_keys":["sk-test"]}"#, &TEST_KEY).unwrap();
        let seed = capability_seed(platform, ModelTask::ImageGeneration);
        let capabilities = [capability_input(&seed, "{}")];
        repo.create(
            CreateProviderParams {
                provider_id: None,
                platform,
                name: "Wiremock Provider",
                base_url,
                auth_scheme: "bearer",
                credentials_encrypted: &encrypted,
                enabled: true,
                bedrock_config: None,
                sort_order: None,
            },
            &NewProviderModel {
                model: "__test_seed__",
                enabled: false,
                sort_order: i64::MAX,
                description: None,
                capabilities: &capabilities,
            },
            &[],
        )
        .await
        .unwrap()
        .0
        .provider_id
    }

    async fn seed_model(
        pool: &SqlitePool,
        provider_id: &str,
        model: &str,
        tasks: &str,
        params: &str,
        enabled: bool,
    ) {
        let provider = SqliteProviderRepository::new(pool.clone())
            .find_by_id(provider_id)
            .await
            .unwrap()
            .expect("provider exists");
        let tasks: Vec<ModelTask> = serde_json::from_str(tasks).expect("valid task list");
        let seeds: Vec<_> = tasks
            .into_iter()
            .map(|task| capability_seed(&provider.platform, task))
            .collect();
        let capabilities: Vec<_> = seeds
            .iter()
            .map(|seed| capability_input(seed, params))
            .collect();
        let repo = SqliteProviderModelRepository::new(pool.clone());
        repo.save(
            provider_id,
            provider.config_revision,
            &NewProviderModel {
                model,
                enabled,
                sort_order: 0,
                description: None,
                capabilities: &capabilities,
            },
        )
        .await
        .unwrap();
    }

    fn mref(provider_id: &str, model: &str) -> ModelRef {
        ModelRef { provider_id: provider_id.into(), model: model.into() }
    }

    fn image_request(prompt: &str) -> TaskRequest {
        TaskRequest::ImageGeneration(ImageGenRequest {
            prompt: prompt.into(),
            count: 1,
            size: None,
            quality: None,
            extra: json!({}),
        })
    }

    fn video_request() -> TaskRequest {
        TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "a wave".into(),
            seconds: None,
            size: None,
            inputs: vec![],
            extra: json!({}),
        })
    }

    // -- invoke --------------------------------------------------------------

    #[tokio::test]
    async fn invoke_image_generation_end_to_end() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("authorization", "Bearer sk-test"))
            .and(body_partial_json(json!({"model": "gpt-image-1", "prompt": "a fox", "n": 1})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, "{}", true).await;

        let out = svc.invoke(&mref(&pid, "gpt-image-1"), image_request("a fox")).await.unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = out else { panic!("expected Done(Assets)") };
        assert_eq!(assets.len(), 1);
        assert!(matches!(&assets[0].data, ProducedData::Bytes(b) if b == b"hi"));
    }

    #[tokio::test]
    async fn invocation_context_survives_catalog_disable_for_materialization() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": [{"b64_json": "aGk="}]})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, "{}", true).await;
        let model = mref(&pid, "gpt-image-1");
        let (outcome, context) = svc
            .invoke_with_context(&model, image_request("a fox"))
            .await
            .unwrap();
        let TaskOutcome::Done(TaskResult::Assets(assets)) = outcome else {
            panic!("expected Done(Assets)")
        };

        SqliteProviderRepository::new(pool.clone())
            .update(
                &pid,
                provider_revision(&pool, &pid).await,
                UpdateProviderParams {
                    enabled: Some(false),
                    ..UpdateProviderParams::default()
                },
            )
            .await
            .unwrap();

        let stale_error = svc
            .materialize_assets_for_model(
                &model,
                ModelTask::ImageGeneration,
                assets.clone(),
                crate::MaterializeLimits::default(),
            )
            .await
            .unwrap_err();
        assert_eq!(stale_error.kind, InvokeErrorKind::Config);

        let materialized = svc
            .materialize_assets_for_invocation(
                &context,
                assets,
                crate::MaterializeLimits::default(),
            )
            .await
            .unwrap();
        assert_eq!(materialized[0].bytes, b"hi");
    }

    #[tokio::test]
    async fn invoke_task_mismatch_is_unsupported_task_without_network() {
        let server = MockServer::start().await;
        // No mock mounted: any request reaching the server would 404 — but the
        // gate must reject before the wire.
        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "gpt-4o", r#"["chat"]"#, "{}", true).await;

        let err = svc.invoke(&mref(&pid, "gpt-4o"), image_request("a fox")).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::UnsupportedTask);
        assert!(server.received_requests().await.unwrap().is_empty(), "gate must fire before the wire");
    }

    // -- probe ---------------------------------------------------------------

    #[tokio::test]
    async fn probe_multipart_task_upstream_400_is_unhealthy() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/transcriptions"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                json!({"error": {"message": "file is required", "type": "invalid_request_error"}}),
            ))
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "whisper-1", r#"["speech_recognition"]"#, "{}", true).await;

        let report = svc.probe(&mref(&pid, "whisper-1"), ModelTask::SpeechRecognition).await.unwrap();
        assert!(!report.healthy);
        assert!(report.message.as_deref().is_some_and(|message| message.contains("400")));
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

    #[tokio::test]
    async fn probe_image_edit_upstream_400_is_unhealthy_and_reaches_the_wire() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                json!({"error": {"message": "image is invalid", "type": "invalid_request_error"}}),
            ))
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_edit"]"#, "{}", true).await;

        let report = svc.probe(&mref(&pid, "gpt-image-1"), ModelTask::ImageEdit).await.unwrap();
        assert!(!report.healthy);
        assert!(report.message.as_deref().is_some_and(|message| message.contains("400")));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "the edit probe must actually reach the wire");
    }

    #[tokio::test]
    async fn probe_tts_voice_400_is_unhealthy_and_reaches_the_wire() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": {
                    "message": "The voice_id (alloy) does not exist or you do not have access to it.",
                    "type": "voice_id_invalid"
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "step-tts-mini", r#"["speech_synthesis"]"#, "{}", true).await;

        let report =
            svc.probe(&mref(&pid, "step-tts-mini"), ModelTask::SpeechSynthesis).await.unwrap();
        assert!(!report.healthy);
        assert!(report.message.as_deref().is_some_and(|message| message.contains("400")));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "the tts probe must actually reach the wire");
    }

    /// StepFun rejects a voice-less TTS request locally, so before the
    /// protocol-scoped fallback the probe failed with `InvalidParams` in a few
    /// milliseconds without ever opening a socket — a correctly configured
    /// model looked broken. The previous TTS probe test seeded platform
    /// `openai`, so it never covered this path.
    #[tokio::test]
    async fn probe_stepfun_tts_without_configured_voice_still_reaches_the_wire() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(body_partial_json(json!({
                "model": "step-tts-mini",
                "input": "hi",
                "voice": "cixingnansheng",
            })))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(b"ID3-audio".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let base = format!("{}/v1", server.uri().trim_end_matches('/'));
        let pid = seed_provider_on(&pool, "stepfun", &base).await;
        // provider_params "{}" is exactly the shipped default: no voice.
        seed_model(&pool, &pid, "step-tts-mini", r#"["speech_synthesis"]"#, "{}", true).await;

        let report =
            svc.probe(&mref(&pid, "step-tts-mini"), ModelTask::SpeechSynthesis).await.unwrap();

        assert!(report.healthy, "unexpected probe failure: {:?}", report.message);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "the stepfun tts probe must actually reach the wire");
    }

    /// The catalog voice must win over the fallback: the fallback exists only
    /// to keep an unconfigured model probeable, never to override a choice.
    #[tokio::test]
    async fn probe_stepfun_tts_prefers_the_configured_voice_over_the_fallback() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/audio/speech"))
            .and(body_partial_json(json!({"voice": "tianmeinvsheng"})))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "audio/mpeg")
                    .set_body_bytes(b"ID3-audio".to_vec()),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let base = format!("{}/v1", server.uri().trim_end_matches('/'));
        let pid = seed_provider_on(&pool, "stepfun", &base).await;
        seed_model(
            &pool,
            &pid,
            "step-tts-mini",
            r#"["speech_synthesis"]"#,
            r#"{"voice":"tianmeinvsheng"}"#,
            true,
        )
        .await;

        let report =
            svc.probe(&mref(&pid, "step-tts-mini"), ModelTask::SpeechSynthesis).await.unwrap();

        assert!(report.healthy, "unexpected probe failure: {:?}", report.message);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    /// A blank configured voice is broken configuration, not an absent one.
    /// Substituting the fallback there would green the badge while every real
    /// synthesis call keeps failing on the same stored value.
    #[tokio::test]
    async fn probe_stepfun_tts_does_not_paper_over_a_blank_configured_voice() {
        let (svc, pool) = setup().await;
        let pid = seed_provider_on(&pool, "stepfun", "http://127.0.0.1:1/v1").await;
        seed_model(
            &pool,
            &pid,
            "step-tts-mini",
            r#"["speech_synthesis"]"#,
            r#"{"voice":"   "}"#,
            true,
        )
        .await;

        let report =
            svc.probe(&mref(&pid, "step-tts-mini"), ModelTask::SpeechSynthesis).await.unwrap();

        assert!(!report.healthy);
        assert!(
            report.message.as_deref().is_some_and(|message| message.contains("voice")),
            "unexpected message: {:?}",
            report.message
        );
    }

    #[tokio::test]
    async fn probe_image_edit_unreachable_endpoint_is_unhealthy() {
        // A dead endpoint must NOT be classified healthy: the local/transport
        // failure has no http_status, so the tolerance arm does not fire.
        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, "http://127.0.0.1:1").await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_edit"]"#, "{}", true).await;

        let report = svc.probe(&mref(&pid, "gpt-image-1"), ModelTask::ImageEdit).await.unwrap();
        assert!(!report.healthy, "a connection-refused probe must be unhealthy");
        assert!(report.message.is_some());
    }

    #[tokio::test]
    async fn probe_500_is_unhealthy_with_message() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, "{}", true).await;

        let report = svc.probe(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration).await.unwrap();
        assert!(!report.healthy);
        let message = report.message.expect("unhealthy probe carries a message");
        assert!(message.contains("500"), "message: {message}");
    }

    #[tokio::test]
    async fn probe_400_on_json_task_stays_unhealthy() {
        // The reachable-only tolerance covers only the placeholder-bearing tasks
        // (ImageEdit / SpeechRecognition stub file, SpeechSynthesis voice id); a
        // 400 on a plain JSON task like image generation is a real failure.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                json!({"error": {"message": "prompt rejected", "type": "invalid_request_error"}}),
            ))
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, "{}", true).await;

        let report = svc.probe(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration).await.unwrap();
        assert!(!report.healthy);
        assert!(report.message.unwrap().contains("400"));
    }

    #[tokio::test]
    async fn probe_video_pending_is_healthy_and_never_polls() {
        // An async-accepted submit is healthy on its own; the probe must not
        // follow up with poll or content requests.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "v1", "status": "queued"})))
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "sora-2", r#"["video_generation"]"#, "{}", true).await;

        let report = svc.probe(&mref(&pid, "sora-2"), ModelTask::VideoGeneration).await.unwrap();
        assert!(report.healthy, "message: {:?}", report.message);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "probe must stop at submit: no poll, no content download");
    }

    #[tokio::test]
    async fn probe_overlays_catalog_params_like_minimal_json_body() {
        // minimal_json_body mirror: catalog params (size/quality directly,
        // steps via the whitelisted extra passthrough) ride the minimal
        // "health check" request so providers requiring them validate.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(body_partial_json(json!({
                "prompt": "health check",
                "n": 1,
                "size": "512x512",
                "quality": "high",
                "steps": 20,
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"b64_json": "aGk="}]})))
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(
            &pool,
            &pid,
            "gpt-image-1",
            r#"["image_generation"]"#,
            r#"{"size": "512x512", "quality": "high", "steps": 20}"#,
            true,
        )
        .await;

        let report = svc.probe(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration).await.unwrap();
        assert!(report.healthy, "message: {:?}", report.message);
    }

    #[tokio::test]
    async fn probe_disabled_model_is_unhealthy_with_model_disabled_message() {
        // Pinned T2 decision: resolve(enforce=false) still rejects disabled
        // rows, so probing a disabled model reports unhealthy — it does not
        // silently probe past the operator's kill switch.
        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, "https://unused.example").await;
        seed_model(&pool, &pid, "gpt-image-1", r#"["image_generation"]"#, "{}", false).await;

        let report = svc.probe(&mref(&pid, "gpt-image-1"), ModelTask::ImageGeneration).await.unwrap();
        assert!(!report.healthy);
        let message = report.message.expect("unhealthy probe carries a message");
        assert!(message.contains("model disabled"), "message: {message}");
    }

    #[tokio::test]
    async fn probe_chat_reports_unsupported_one_shot_request() {
        let (svc, _pool) = setup().await;
        let report = svc.probe(&mref("any", "gpt-4o"), ModelTask::Chat).await.unwrap();
        assert!(!report.healthy);
        assert!(report.message.as_deref().is_some_and(|message| message.contains("one-shot")));
    }

    // -- poll ----------------------------------------------------------------

    #[tokio::test]
    async fn poll_rides_job_adapter_id_and_hits_status_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/v1"))
            .and(header("authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "v1", "status": "in_progress"})))
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "sora-2", r#"["video_generation"]"#, "{}", true).await;

        let job = JobHandle {
            adapter_id: "openai.videos".into(),
            config_revision: provider_revision(&pool, &pid).await,
            remote_id: "v1".into(),
            poll_state: json!({}),
        };
        let out = svc.poll(&mref(&pid, "sora-2"), video_request(), &job).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected Pending") };
        assert_eq!(handle.remote_id, "v1");
        assert_eq!(handle.adapter_id, "openai.videos");
    }

    #[tokio::test]
    async fn invocation_context_polls_after_catalog_disable() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/videos"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"id": "v1", "status": "queued"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/v1"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id": "v1", "status": "in_progress"})),
            )
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "sora-2", r#"["video_generation"]"#, "{}", true).await;
        let model = mref(&pid, "sora-2");
        let (submitted, context) = svc
            .invoke_with_context(&model, video_request())
            .await
            .unwrap();
        let TaskOutcome::Pending(job) = submitted else {
            panic!("expected Pending")
        };
        SqliteProviderRepository::new(pool.clone())
            .update(
                &pid,
                provider_revision(&pool, &pid).await,
                UpdateProviderParams {
                    enabled: Some(false),
                    ..UpdateProviderParams::default()
                },
            )
            .await
            .unwrap();

        let legacy_error = svc
            .poll(&model, video_request(), &job)
            .await
            .unwrap_err();
        assert_eq!(legacy_error.kind, InvokeErrorKind::Config);
        let polled = svc
            .poll_with_context(&context, video_request(), &job)
            .await
            .unwrap();
        assert!(matches!(polled, TaskOutcome::Pending(_)));
    }

    #[tokio::test]
    async fn poll_requires_the_same_task_capability() {
        let server = MockServer::start().await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "sora-2", r#"["chat"]"#, "{}", true).await;

        let job = JobHandle {
            adapter_id: "openai.videos".into(),
            config_revision: provider_revision(&pool, &pid).await,
            remote_id: "v1".into(),
            poll_state: json!({}),
        };
        let error = svc.poll(&mref(&pid, "sora-2"), video_request(), &job).await.unwrap_err();
        assert_eq!(error.kind, InvokeErrorKind::UnsupportedTask);
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn poll_rejects_a_job_protocol_that_differs_from_the_capability() {
        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, "https://unused.example").await;
        seed_model(&pool, &pid, "sora-2", r#"["video_generation"]"#, "{}", true).await;

        let job = JobHandle {
            adapter_id: "ghost.videos".into(),
            config_revision: provider_revision(&pool, &pid).await,
            remote_id: "v1".into(),
            poll_state: json!({}),
        };
        let err = svc.poll(&mref(&pid, "sora-2"), video_request(), &job).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Config);
        assert!(err.message.contains("protocol"), "message: {}", err.message);
    }

    // -- multi-connection profiles (the P1 architecture's acceptance test) ----

    fn asr_request() -> TaskRequest {
        TaskRequest::SpeechRecognition(AsrRequest {
            audio: InputAsset { id: None, role: "audio".into(), bytes: b"RIFFdata".to_vec(), mime: "audio/wav".into() },
            language: None,
            prompt: None,
            extra: json!({}),
        })
    }

    #[tokio::test]
    async fn volc_asr_rides_voice_connection_not_default_end_to_end() {
        // THE multi-connection acceptance test: one provider, two domains, two
        // credential sets. The Ark default connection points at wiremock A;
        // the "voice" connection profile points at wiremock B with its own
        // volc_voice credentials. SpeechRecognition must ride B exclusively —
        // wiremock A never sees a single request.
        let ark_server = MockServer::start().await; // A — default connection
        let voice_server = MockServer::start().await; // B — "voice" profile

        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/submit"))
            .and(header("X-Api-App-Key", "voice-app"))
            .and(header("X-Api-Access-Key", "voice-ak"))
            .and(header("X-Api-Resource-Id", "volc.bigasr.auc"))
            .respond_with(ResponseTemplate::new(200).insert_header("X-Api-Status-Code", "20000000"))
            .expect(1)
            .mount(&voice_server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v3/auc/bigmodel/query"))
            .and(header("X-Api-App-Key", "voice-app"))
            .and(header("X-Api-Access-Key", "voice-ak"))
            .and(header("X-Api-Resource-Id", "volc.bigasr.auc"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("X-Api-Status-Code", "20000000")
                    .set_body_json(json!({"result": {"text": "hello volc"}})),
            )
            .expect(1)
            .mount(&voice_server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider_on(&pool, "ark", &ark_server.uri()).await;
        let conn_repo = SqliteProviderConnectionRepository::new(pool.clone());
        let creds = r#"{"app_key":"voice-app","access_key":"voice-ak","resource_id":"volc.bigasr.auc"}"#;
        conn_repo
            .upsert(
                &pid,
                provider_revision(&pool, &pid).await,
                &UpsertProviderConnectionParams {
                    role: "voice",
                    label: Some("Voice"),
                    base_url: &voice_server.uri(),
                    auth_scheme: "volc_voice",
                    credentials_encrypted: &encrypt_string(creds, &TEST_KEY).unwrap(),
                    extra: "{}",
                },
            )
            .await
            .unwrap();
        seed_model(&pool, &pid, "bigmodel-asr", r#"["speech_recognition"]"#, "{}", true).await;

        // 1) invoke → submit hits wiremock B and returns the pending handle.
        let out = svc.invoke(&mref(&pid, "bigmodel-asr"), asr_request()).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected Pending from volc submit") };
        assert_eq!(handle.adapter_id, "volc.asr_file");
        assert_eq!(handle.poll_state, json!({}));

        // 2) poll with that handle → query hits B, same X-Api-Request-Id value
        //    as the submit, and yields the transcript.
        let out = svc.poll(&mref(&pid, "bigmodel-asr"), asr_request(), &handle).await.unwrap();
        let TaskOutcome::Done(TaskResult::Transcript { text, model, .. }) = out else {
            panic!("expected Done(Transcript)")
        };
        assert_eq!(text, "hello volc");
        assert_eq!(model.as_deref(), Some("bigmodel-asr"));

        let voice_requests = voice_server.received_requests().await.unwrap();
        assert_eq!(voice_requests.len(), 2, "submit + query, both on the voice domain");
        let ids: Vec<&str> = voice_requests
            .iter()
            .map(|r| r.headers.get("X-Api-Request-Id").expect("request id header").to_str().unwrap())
            .collect();
        assert_eq!(ids[0], handle.remote_id, "submit carries the client-generated id");
        assert_eq!(ids[0], ids[1], "query reuses the submit request id verbatim");

        // 3) Delete the voice profile → the same call is now MissingConnection.
        let error = conn_repo.delete(&pid, "voice").await.unwrap_err();
        assert!(error.to_string().contains("still referenced"));

        // 4) Per-task connection isolation: the default (Ark) connection was
        //    never touched by any of the above.
        assert!(
            ark_server.received_requests().await.unwrap().is_empty(),
            "voice traffic must never leak onto the default Ark connection"
        );
    }
}
