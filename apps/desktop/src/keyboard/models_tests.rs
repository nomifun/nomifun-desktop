use super::*;
use nomifun_db::{
    CreateProviderParams, IProviderRepository, NewProviderModel, NewProviderModelCapability,
    SqliteProviderConnectionRepository, SqliteProviderModelCapabilityRepository,
    SqliteProviderModelRepository, SqliteProviderRepository,
};
use nomifun_model_invoke::{
    AdapterRegistry, InvokeError, JobHandle, ProtocolAdapter, ResolvedCall,
};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

#[test]
fn selection_uses_existing_catalog_identity_and_instruction_retains_invariants() {
    let selected: KeyboardModelSelection = serde_json::from_value(
        serde_json::json!({"providerId":"exact-provider","model":"exact-model"}),
    )
    .unwrap();
    assert_eq!(selected.model_ref().provider_id, "exact-provider");
    let instruction = polish_instruction("整理标点和段落");
    assert!(instruction.contains("保留原意、事实、人名、数字、日期、代码和专有词"));
    assert!(instruction.ends_with("整理标点和段落"));
}

#[test]
fn wav_keeps_pcm_exact_and_protocol_modes_remain_disjoint() {
    let pcm = [-32768, -42, 0, 42, 32767];
    let wav = pcm16_mono_16k_wav(&pcm).unwrap();
    validate_platform_wav(&wav).unwrap();
    let decoded: Vec<_> = wav[44..]
        .chunks_exact(2)
        .map(|s| i16::from_le_bytes(s.try_into().unwrap()))
        .collect();
    assert_eq!(decoded, pcm);
    assert!(pcm16_mono_16k_wav(&[]).is_err());
    let mut malformed = wav.clone();
    malformed[24..28].copy_from_slice(&44100u32.to_le_bytes());
    assert!(validate_platform_wav(&malformed).is_err());
    assert!(validate_platform_wav(&wav[..wav.len() - 1]).is_err());
    assert!(is_platform_asr_protocol("stepfun.asr_sse"));
    assert!(is_platform_asr_protocol("volc.asr_file"));
    assert!(!is_platform_asr_protocol("stepfun.asr_stream"));
    assert!(!is_platform_asr_protocol("openai.chat_text"));
}

async fn fixture(
    base: &str,
    registry: AdapterRegistry,
) -> (
    DesktopModels,
    KeyboardModelSelection,
    KeyboardModelSelection,
) {
    let db = nomifun_db::init_database_memory().await.unwrap();
    let pool = db.pool().clone();
    let key = [0x31; 32];
    let encrypted =
        nomifun_common::encrypt_string(r#"{"api_keys":["local-asr-fixture"]}"#, &key).unwrap();
    let repo = SqliteProviderRepository::new(pool.clone());
    let file_caps = [NewProviderModelCapability {
        task: "speech_recognition",
        traits: "[]",
        protocol: "stepfun.asr_sse",
        connection_role: "default",
        endpoint: Some("/audio/asr/sse"),
        provider_params: "{}",
        ..Default::default()
    }];
    let live_caps = [NewProviderModelCapability {
        task: "speech_recognition",
        traits: "[]",
        protocol: "stepfun.asr_stream",
        connection_role: "default",
        realtime_endpoint: Some("/realtime/asr/stream"),
        provider_params: "{}",
        ..Default::default()
    }];
    let configured = [
        NewProviderModel {
            model: "stepaudio-2.5-asr",
            enabled: true,
            capabilities: &file_caps,
            ..Default::default()
        },
        NewProviderModel {
            model: "stepaudio-2.5-asr-stream",
            enabled: true,
            capabilities: &live_caps,
            ..Default::default()
        },
    ];
    let (provider, _) = repo
        .create_graph(
            CreateProviderParams {
                provider_id: None,
                platform: "stepfun",
                name: "isolated model fixture",
                base_url: base,
                auth_scheme: "bearer",
                credentials_encrypted: &encrypted,
                enabled: true,
                bedrock_config: None,
                sort_order: None,
            },
            &configured,
            &[None, None],
            &[],
        )
        .await
        .unwrap();
    let invoke = Arc::new(ModelInvokeService::new(
        Arc::new(repo),
        Arc::new(SqliteProviderModelRepository::new(pool.clone())),
        Arc::new(SqliteProviderModelCapabilityRepository::new(pool.clone())),
        Arc::new(SqliteProviderConnectionRepository::new(pool)),
        key,
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
        registry,
    ));
    let models = DesktopModels::new(invoke);
    (
        models,
        KeyboardModelSelection {
            provider_id: provider.provider_id.clone(),
            model: "stepaudio-2.5-asr".into(),
        },
        KeyboardModelSelection {
            provider_id: provider.provider_id,
            model: "stepaudio-2.5-asr-stream".into(),
        },
    )
}
fn config() -> StreamingAsrConfig {
    StreamingAsrConfig {
        language: Some("zh".into()),
        dictionary: vec!["项目进度".into()],
    }
}
fn wave() -> Vec<u8> {
    pcm16_mono_16k_wav(
        &(0..1600)
            .map(|n| if n % 2 == 0 { 1000 } else { -1000 })
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

#[tokio::test]
async fn platform_upload_preserves_config_and_rejects_failure_cancel_and_late_result() {
    use axum::{
        Json, Router,
        body::{Body, Bytes},
        http::Response,
        routing::post,
    };
    use base64::Engine as _;
    let mode = Arc::new(AtomicU8::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let observed = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
    let handler_mode = mode.clone();
    let handler_calls = calls.clone();
    let handler_started = started.clone();
    let handler_release = release.clone();
    let handler_observed = observed.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = Router::new().route(
        "/audio/asr/sse",
        post(move |Json(body): Json<serde_json::Value>| {
            let mode = handler_mode.load(Ordering::Acquire);
            let calls = handler_calls.clone();
            let started = handler_started.clone();
            let release = handler_release.clone();
            let observed = handler_observed.clone();
            async move {
                calls.fetch_add(1, Ordering::AcqRel);
                observed.lock().unwrap().push(body);
                if mode == 3 {
                    started.notify_one();
                    release.notified().await;
                }
                if mode == 1 {
                    return Response::builder()
                        .status(500)
                        .body(Body::from(r#"{"error":{"message":"fixture failed"}}"#))
                        .unwrap();
                }
                if mode == 2 {
                    let chunks = vec![
                        Ok::<_, std::io::Error>(Bytes::from_static(
                            b"event: transcript.text.delta\ndata: {\"delta\":\"partial\"}\n\n",
                        )),
                        Err(std::io::Error::new(
                            std::io::ErrorKind::UnexpectedEof,
                            "fixture transport truncated",
                        )),
                    ];
                    return Response::builder()
                        .header("content-type", "text/event-stream")
                        .body(Body::from_stream(futures_util::stream::iter(chunks)))
                        .unwrap();
                }
                let result = if mode == 4 {
                    "event: error\ndata: {\"error\":{\"message\":\"fixture failed\"}}\n\n"
                } else {
                    "event: transcript.text.done\ndata: {\"text\":\"完整结果\"}\n\n"
                };
                Response::builder()
                    .header("content-type", "text/event-stream")
                    .body(Body::from(result))
                    .unwrap()
            }
        }),
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let (models, file, live) = fixture(
        &base,
        AdapterRegistry::new(nomifun_model_invoke::default_adapters()),
    )
    .await;
    let catalog = models.catalog().await.unwrap();
    assert_eq!(catalog.asr.len(), 1);
    assert_eq!(catalog.platform_asr.len(), 1);
    assert_eq!(catalog.asr[0].model, live.model);
    assert_eq!(catalog.platform_asr[0].model, file.model);
    assert!(
        models
            .transcribe_platform(&live, wave(), config(), CancellationToken::new())
            .await
            .is_err()
    );
    assert!(models.open_asr(&file, config()).await.is_err());
    assert_eq!(calls.load(Ordering::Acquire), 0);
    let result = models
        .transcribe_platform(&file, wave(), config(), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(result, "完整结果");
    let body = observed.lock().unwrap()[0].clone();
    assert!(body.get("tools").is_none() && body.get("messages").is_none());
    assert_eq!(body["audio"]["input"]["transcription"]["language"], "zh");
    assert!(
        body["audio"]["input"]["transcription"]["prompt"]
            .as_str()
            .unwrap()
            .contains("项目进度")
    );
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(body["audio"]["data"].as_str().unwrap())
            .unwrap(),
        wave()
    );
    for failure_mode in [1, 2, 4] {
        mode.store(failure_mode, Ordering::Release);
        assert!(
            models
                .transcribe_platform(&file, wave(), config(), CancellationToken::new())
                .await
                .is_err()
        );
    }
    let before = calls.load(Ordering::Acquire);
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(
        models
            .transcribe_platform(&file, wave(), config(), cancelled)
            .await
            .unwrap_err()
            .contains("取消")
    );
    assert_eq!(calls.load(Ordering::Acquire), before);
    mode.store(3, Ordering::Release);
    let cancel = CancellationToken::new();
    let mut request = Box::pin(models.transcribe_platform(&file, wave(), config(), cancel.clone()));
    tokio::select! {_=&mut request=>panic!("fixture must wait"),_=started.notified()=>{}}
    release.notify_one();
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();
    assert!(request.await.unwrap_err().contains("取消"));
    assert_eq!(calls.load(Ordering::Acquire), before + 1);
    server.abort();
}

struct PendingAsr {
    submit: Arc<AtomicUsize>,
    poll: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl ProtocolAdapter for PendingAsr {
    fn id(&self) -> &'static str {
        "stepfun.asr_sse"
    }
    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::SpeechRecognition
    }
    async fn submit(
        &self,
        _: &reqwest::Client,
        _: &ResolvedCall,
    ) -> Result<TaskOutcome, InvokeError> {
        self.submit.fetch_add(1, Ordering::AcqRel);
        Ok(TaskOutcome::Pending(JobHandle {
            adapter_id: self.id().into(),
            config_revision: 0,
            remote_id: "fixture".into(),
            poll_state: serde_json::json!({}),
        }))
    }
    async fn poll(
        &self,
        _: &reqwest::Client,
        _: &ResolvedCall,
        job: &JobHandle,
    ) -> Result<TaskOutcome, InvokeError> {
        if self.poll.fetch_add(1, Ordering::AcqRel) == 0 {
            Ok(TaskOutcome::Pending(job.clone()))
        } else {
            Ok(TaskOutcome::Done(TaskResult::Transcript {
                text: "任务定稿".into(),
                language: None,
                model: None,
            }))
        }
    }
}
#[tokio::test]
async fn platform_pending_jobs_are_polled_after_a_single_submission() {
    let submit = Arc::new(AtomicUsize::new(0));
    let poll = Arc::new(AtomicUsize::new(0));
    let (models, file, _) = fixture(
        "http://127.0.0.1:1",
        AdapterRegistry::new(vec![Arc::new(PendingAsr {
            submit: submit.clone(),
            poll: poll.clone(),
        })]),
    )
    .await;
    assert_eq!(
        models
            .transcribe_platform(&file, wave(), config(), CancellationToken::new())
            .await
            .unwrap(),
        "任务定稿"
    );
    assert_eq!(submit.load(Ordering::Acquire), 1);
    assert_eq!(poll.load(Ordering::Acquire), 2);
}
