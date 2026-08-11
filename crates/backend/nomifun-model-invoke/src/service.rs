//! [`ModelInvokeService`] — the invoke layer's entry point: catalog
//! repositories + credential decryption key + shared HTTP client + the
//! protocol adapter registry. This module carries the constructor and the
//! invoke / poll / probe flows; the catalog resolution pipeline they
//! all share lives in [`crate::resolve`].

use std::sync::Arc;
use std::time::{Duration, Instant};

use nomifun_api_types::ModelTask;
use serde_json::json;

use crate::adapter::AdapterRegistry;
use crate::error::{InvokeError, InvokeErrorKind};
use crate::types::{
    AsrRequest, EmbedRequest, ImageEditRequest, ImageGenRequest, InputAsset, JobHandle, ModelRef,
    TaskOutcome, TaskRequest, TtsRequest, VideoGenRequest,
};

/// Ceiling on one modality probe (resolution + submit), matching the legacy
/// `provider_health` modality probe.
const PROBE_TIMEOUT: Duration = Duration::from_secs(60);

/// A minimal valid 1x1 RGBA PNG (67 bytes) used as the ImageEdit probe's stub
/// input: `openai.images` rejects an input-less edit locally (never reaching
/// the wire), so the probe must carry a real decodable image for the request
/// to exercise endpoint + auth + model.
const PROBE_PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
    0x00, 0x00, 0x00, 0x0D, b'I', b'H', b'D', b'R', // IHDR (13 bytes)
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, // 1x1
    0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, // 8-bit RGBA + CRC
    0x89, //
    0x00, 0x00, 0x00, 0x0A, b'I', b'D', b'A', b'T', // IDAT (10 bytes)
    0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, // zlib: one transparent px
    0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, // data end + CRC
    0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', // IEND
    0xAE, 0x42, 0x60, 0x82, // CRC
];

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
    pub(crate) provider_connection_repo: Arc<dyn nomifun_db::IProviderConnectionRepository>,
    /// AES-256-GCM key used to decrypt stored provider/connection credentials.
    pub(crate) encryption_key: [u8; 32],
    /// Shared client for all adapter calls.
    pub(crate) http: reqwest::Client,
    pub(crate) registry: AdapterRegistry,
}

impl ModelInvokeService {
    pub fn new(
        provider_repo: Arc<dyn nomifun_db::IProviderRepository>,
        provider_model_repo: Arc<dyn nomifun_db::IProviderModelRepository>,
        provider_connection_repo: Arc<dyn nomifun_db::IProviderConnectionRepository>,
        encryption_key: [u8; 32],
        http: reqwest::Client,
        registry: AdapterRegistry,
    ) -> Self {
        Self { provider_repo, provider_model_repo, provider_connection_repo, encryption_key, http, registry }
    }

    /// The provider repository this service resolves against. Shared with
    /// callers (e.g. the creation engine) that pre-check provider existence
    /// before enqueueing work, so they don't need a second repo handle.
    pub fn provider_repo(&self) -> &Arc<dyn nomifun_db::IProviderRepository> {
        &self.provider_repo
    }

    /// Execute one task invocation: full resolution (task-membership gate
    /// enforced) then the adapter's submit. A [`TaskOutcome::Pending`] hands
    /// back a [`JobHandle`] the caller later feeds to [`Self::poll`].
    pub async fn invoke(&self, m: &ModelRef, req: TaskRequest) -> Result<TaskOutcome, InvokeError> {
        let task = req.task();
        let (call, adapter) = self.resolve(m, task, req, true).await?;
        adapter.submit(&self.http, &call).await
    }

    /// Poll a pending job. Resolution runs with `enforce_task_membership =
    /// false` — recovery semantics: the model's task tags may have changed
    /// since submit, and polling must not re-gate an already-accepted job.
    /// The adapter is taken DIRECTLY from the registry by `job.adapter_id`
    /// (the job pins its protocol; no route/row re-derivation), so an
    /// unregistered `adapter_id` is an honest NoAdapter.
    pub async fn poll(
        &self,
        m: &ModelRef,
        req: TaskRequest,
        job: &JobHandle,
    ) -> Result<TaskOutcome, InvokeError> {
        let task = req.task();
        // Resolved connection/params ride the CURRENT catalog state; the
        // resolver's own adapter pick is deliberately discarded below.
        let (call, _route_adapter) = self.resolve(m, task, req, false).await?;
        let adapter = self.registry.get(&job.adapter_id, task)?;
        adapter.poll(&self.http, &call, job).await
    }

    /// Health-probe `(model, task)` with the smallest valid request (a port of
    /// the legacy `provider_health::run_modality_probe` semantics):
    ///
    /// - resolution runs un-gated (`enforce_task_membership = false`) — probing
    ///   an untagged model must not be blocked by catalog membership (disabled
    ///   provider/model rows still refuse, and fold into `healthy = false`);
    /// - `Done` OR `Pending` → healthy (an async accept proves the endpoint;
    ///   the probe never polls and never downloads content);
    /// - multipart tasks (ImageEdit / SpeechRecognition) carry no real file, so
    ///   an [`InvokeErrorKind::InvalidParams`] (missing-file 400) still proves
    ///   endpoint + auth + model are reachable → healthy;
    /// - any other error → `healthy = false` with the error text;
    /// - the whole attempt is capped at 60 s.
    ///
    /// Chat is refused up front ([`InvokeErrorKind::Config`]): chat probes run
    /// through the agent engine, not the invoke layer.
    pub async fn probe(&self, m: &ModelRef, task: ModelTask) -> Result<ProbeReport, InvokeError> {
        if task == ModelTask::Chat {
            return Err(InvokeError::config(
                "chat probes run through the agent engine, not the invoke layer",
            ));
        }
        let started = Instant::now();
        let attempt = async {
            // Two-phase build: resolve with a params-free placeholder, then
            // rebuild the minimal request from the resolved model_params so
            // catalog defaults (image size/quality, TTS voice, …) ride along —
            // the mirror of the legacy prober's minimal_json_body overlay.
            let placeholder = probe_request(task, &json!({}));
            let (mut call, adapter) = self.resolve(m, task, placeholder, false).await?;
            call.request = probe_request(task, &call.model_params);
            adapter.submit(&self.http, &call).await
        };
        let outcome = tokio::time::timeout(PROBE_TIMEOUT, attempt).await;
        let latency_ms = started.elapsed().as_millis() as u64;
        let report = match outcome {
            // Sync success, or async accepted — either proves the model
            // answers. Pending is NOT followed up: no poll, no download.
            Ok(Ok(TaskOutcome::Done(_) | TaskOutcome::Pending(_))) => {
                ProbeReport { healthy: true, latency_ms, message: None }
            }
            // Reachable-only tolerance for probes whose request MUST carry a
            // placeholder the probe cannot make valid generically: the stub
            // file for ImageEdit/SpeechRecognition, and the VOICE ID for
            // SpeechSynthesis — a provider-specific enum the generic probe
            // cannot know, so the OpenAI-family default "alloy" is rejected by
            // providers with their own voice catalog (e.g. StepFun returns 400
            // `voice_id_invalid`). An UPSTREAM InvalidParams (4xx — http_status
            // set) still proves endpoint + auth + model are reachable; the real
            // voice is chosen where it is used (the companion TTS slot / global
            // TTS preference). A LOCAL InvalidParams (http_status None, an
            // adapter pre-flight rejection) never touched the wire and proves
            // nothing — that stays unhealthy.
            Ok(Err(e))
                if matches!(
                    task,
                    ModelTask::ImageEdit
                        | ModelTask::SpeechRecognition
                        | ModelTask::SpeechSynthesis
                ) && e.kind == InvokeErrorKind::InvalidParams
                    && e.http_status.is_some() =>
            {
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
}

/// The smallest valid [`TaskRequest`] for a probe, overlaying known catalog
/// params (image size/quality + steps/cfg_scale/text_mode passthrough, TTS
/// voice) — the typed mirror of the legacy prober's `minimal_json_body` /
/// `minimal_multipart_form`.
fn probe_request(task: ModelTask, params: &serde_json::Value) -> TaskRequest {
    match task {
        ModelTask::ImageGeneration => TaskRequest::ImageGeneration(ImageGenRequest {
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
        }),
        ModelTask::ImageEdit => TaskRequest::ImageEdit(ImageEditRequest {
            prompt: "health check".into(),
            count: 1,
            size: None,
            // A real (minimal) PNG: openai.images rejects an input-less edit
            // locally, and a probe that never reaches the wire is vacuous.
            // The stub is not meaningful content — an upstream 4xx still
            // counts as healthy (reachable-only rule above).
            inputs: vec![InputAsset {
                id: None,
                role: "reference".into(),
                bytes: PROBE_PNG.to_vec(),
                mime: "image/png".into(),
            }],
            extra: json!({}),
        }),
        ModelTask::VideoGeneration => TaskRequest::VideoGeneration(VideoGenRequest {
            prompt: "health check".into(),
            seconds: None,
            size: None,
            inputs: vec![],
            extra: json!({}),
        }),
        ModelTask::SpeechSynthesis => TaskRequest::SpeechSynthesis(TtsRequest {
            text: "hi".into(),
            // From the catalog params when set; otherwise None → the adapter's
            // default voice ("alloy" for the openai family). That default is
            // not valid for every provider, so the probe treats an upstream
            // voice-rejection 400 as reachable (see the tolerance arm above).
            voice: params.get("voice").and_then(|v| v.as_str()).map(str::to_string),
            format: None,
            extra: json!({}),
        }),
        ModelTask::SpeechRecognition => TaskRequest::SpeechRecognition(AsrRequest {
            // Empty audio — reachable-only, same rule as ImageEdit.
            audio: InputAsset { id: None, role: "audio".into(), bytes: vec![], mime: "audio/wav".into() },
            language: None,
            prompt: None,
            extra: json!({}),
        }),
        ModelTask::Embedding => {
            TaskRequest::Embedding(EmbedRequest { inputs: vec!["health check".into()], extra: json!({}) })
        }
        // Rerank has no TaskRequest shape yet; the probe still flows through
        // resolution so the registry's NoAdapter is the honest "nothing serves
        // rerank" signal — this placeholder payload is never submitted. Chat
        // is unreachable here (guarded in `probe`), folded in for exhaustiveness.
        ModelTask::Rerank | ModelTask::Chat => {
            TaskRequest::Embedding(EmbedRequest { inputs: vec!["health check".into()], extra: json!({}) })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use nomifun_api_types::ModelTask;
    use nomifun_common::encrypt_string;
    use nomifun_db::{
        CreateProviderParams, IProviderConnectionRepository, IProviderModelRepository,
        IProviderRepository, NewProviderModel, SqliteProviderConnectionRepository,
        SqliteProviderModelRepository, SqliteProviderRepository, SqlitePool,
        UpsertProviderConnectionParams, init_database_memory,
    };
    use serde_json::json;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::adapters::default_adapters;
    use crate::error::InvokeErrorKind;
    use crate::types::{
        ImageGenRequest, JobHandle, ModelRef, ProducedData, TaskOutcome, TaskRequest, TaskResult,
        VideoGenRequest,
    };

    const TEST_KEY: [u8; 32] = [0x42; 32];

    /// Real in-memory DB + the production adapter set: these are the invoke
    /// layer's end-to-end tests (catalog rows → resolution → wire → outcome).
    async fn setup() -> (ModelInvokeService, SqlitePool) {
        let db = init_database_memory().await.unwrap();
        let pool = db.pool().clone();
        std::mem::forget(db);
        let service = ModelInvokeService::new(
            Arc::new(SqliteProviderRepository::new(pool.clone())),
            Arc::new(SqliteProviderModelRepository::new(pool.clone())),
            Arc::new(SqliteProviderConnectionRepository::new(pool.clone())),
            TEST_KEY,
            reqwest::Client::new(),
            AdapterRegistry::new(default_adapters()),
        );
        (service, pool)
    }

    /// Seed an enabled openai-platform provider whose base_url is the mock
    /// server (key decrypts to `sk-test` → `Authorization: Bearer sk-test`).
    async fn seed_provider(pool: &SqlitePool, base_url: &str) -> String {
        seed_provider_on(pool, "openai", base_url).await
    }

    /// Platform-aware provider seeder (same key material as [`seed_provider`]).
    async fn seed_provider_on(pool: &SqlitePool, platform: &str, base_url: &str) -> String {
        let repo = SqliteProviderRepository::new(pool.clone());
        let encrypted = encrypt_string("sk-test", &TEST_KEY).unwrap();
        repo.create(CreateProviderParams {
            provider_id: None,
            platform,
            name: "Wiremock Provider",
            base_url,
            api_key_encrypted: &encrypted,
            models: "[]",
            enabled: true,
            model_context_limits: None,
            model_protocols: None,
            model_descriptions: None,
            model_enabled: None,
            bedrock_config: None,
            is_full_url: false,
            sort_order: None,
        })
        .await
        .unwrap()
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
        let repo = SqliteProviderModelRepository::new(pool.clone());
        repo.create(
            provider_id,
            &NewProviderModel {
                model,
                enabled,
                sort_order: 0,
                tasks,
                traits: "[]",
                protocol: None,
                params,
                context_limit: None,
                description: None,
                source: "user",
                health: None,
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
    async fn probe_multipart_task_missing_file_400_is_healthy() {
        // Reachable-only rule: the probe sends an empty file, a missing-file
        // 400 proves endpoint + auth + model are reachable (mirrors
        // provider_health's classify for ImageEdit / SpeechRecognition).
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
        assert!(report.healthy, "message: {:?}", report.message);
        assert_eq!(report.message, None);
    }

    #[test]
    fn probe_png_stub_is_a_png() {
        // The ImageEdit probe's stub input must be a decodable PNG so the
        // multipart request survives adapter pre-flight and reaches the wire.
        assert_eq!(&PROBE_PNG[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(PROBE_PNG.len(), 67);
        assert_eq!(&PROBE_PNG[PROBE_PNG.len() - 8..PROBE_PNG.len() - 4], b"IEND");
    }

    #[tokio::test]
    async fn probe_image_edit_upstream_400_is_healthy_and_reaches_the_wire() {
        // The stub PNG carries the edit probe past openai.images' local
        // input-check onto the wire; the upstream missing/invalid-image 400
        // (http_status set) is the reachable-only healthy signal.
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
        assert!(report.healthy, "message: {:?}", report.message);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "the edit probe must actually reach the wire");
    }

    #[tokio::test]
    async fn probe_tts_voice_400_is_healthy_and_reaches_the_wire() {
        // A TTS provider that rejects the probe's placeholder voice (StepFun
        // returns 400 `voice_id_invalid` for the OpenAI-family default "alloy")
        // is reachable — endpoint + auth + model are proven. The real voice is
        // chosen on use (companion TTS slot / global TTS), so the health check
        // must not mark the model unhealthy. Regression for the reported
        // "step-tts-mini: 失败 - InvalidParams ... voice_id (alloy)".
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
        assert!(report.healthy, "a voice-rejection 400 is reachable-only healthy: {:?}", report.message);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "the tts probe must actually reach the wire");
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
    async fn probe_chat_is_config_error_before_resolution() {
        // Chat probes stay on the agent-engine path; the guard fires before
        // any catalog read (no provider seeded here).
        let (svc, _pool) = setup().await;
        let err = svc.probe(&mref("any", "gpt-4o"), ModelTask::Chat).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::Config);
        assert!(err.message.contains("agent engine"), "message: {}", err.message);
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

        let job = JobHandle { adapter_id: "openai.videos".into(), remote_id: "v1".into(), poll_state: json!({}) };
        let out = svc.poll(&mref(&pid, "sora-2"), video_request(), &job).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected Pending") };
        assert_eq!(handle.remote_id, "v1");
        assert_eq!(handle.adapter_id, "openai.videos");
    }

    #[tokio::test]
    async fn poll_does_not_regate_task_membership() {
        // Recovery semantics: the model's tags may have changed since submit;
        // poll resolves with enforce=false, so a row now tagged chat-only
        // still lets the pinned job be polled.
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/videos/v1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "v1", "status": "in_progress"})))
            .expect(1)
            .mount(&server)
            .await;

        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, &server.uri()).await;
        seed_model(&pool, &pid, "sora-2", r#"["chat"]"#, "{}", true).await;

        let job = JobHandle { adapter_id: "openai.videos".into(), remote_id: "v1".into(), poll_state: json!({}) };
        let out = svc.poll(&mref(&pid, "sora-2"), video_request(), &job).await.unwrap();
        assert!(matches!(out, TaskOutcome::Pending(_)));
    }

    #[tokio::test]
    async fn poll_unregistered_job_adapter_is_no_adapter() {
        // The job pins its adapter: an adapter_id nothing registered is a
        // NoAdapter, not a silent protocol re-derivation.
        let (svc, pool) = setup().await;
        let pid = seed_provider(&pool, "https://unused.example").await;
        seed_model(&pool, &pid, "sora-2", r#"["video_generation"]"#, "{}", true).await;

        let job = JobHandle { adapter_id: "ghost.videos".into(), remote_id: "v1".into(), poll_state: json!({}) };
        let err = svc.poll(&mref(&pid, "sora-2"), video_request(), &job).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::NoAdapter);
        assert!(err.message.contains("ghost.videos"), "message: {}", err.message);
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
        // Model row leaves protocol/connection_role NULL: the platform route
        // table alone sends (ark, SpeechRecognition) to volc.asr_file@voice.
        seed_model(&pool, &pid, "bigmodel-asr", r#"["speech_recognition"]"#, "{}", true).await;
        let conn_repo = SqliteProviderConnectionRepository::new(pool.clone());
        let creds = r#"{"app_key":"voice-app","access_key":"voice-ak","resource_id":"volc.bigasr.auc"}"#;
        conn_repo
            .upsert(
                &pid,
                &UpsertProviderConnectionParams {
                    role: "voice",
                    label: Some("Voice"),
                    base_url: &voice_server.uri(),
                    auth_scheme: "volc_voice",
                    credentials_encrypted: &encrypt_string(creds, &TEST_KEY).unwrap(),
                    is_full_url: false,
                    extra: "{}",
                },
            )
            .await
            .unwrap();

        // 1) invoke → submit hits wiremock B and returns the pending handle.
        let out = svc.invoke(&mref(&pid, "bigmodel-asr"), asr_request()).await.unwrap();
        let TaskOutcome::Pending(handle) = out else { panic!("expected Pending from volc submit") };
        assert_eq!(handle.adapter_id, "volc.asr_file");
        assert_eq!(handle.poll_state, json!({"request_id": handle.remote_id}));

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
        assert!(conn_repo.delete(&pid, "voice").await.unwrap());
        let err = svc.invoke(&mref(&pid, "bigmodel-asr"), asr_request()).await.unwrap_err();
        assert_eq!(err.kind, InvokeErrorKind::MissingConnection);
        assert!(err.message.contains("voice"), "message: {}", err.message);

        // 4) Per-task connection isolation: the default (Ark) connection was
        //    never touched by any of the above.
        assert!(
            ark_server.received_requests().await.unwrap().is_empty(),
            "voice traffic must never leak onto the default Ark connection"
        );
    }
}
