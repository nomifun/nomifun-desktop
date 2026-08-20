use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use nomifun_api_types::{
    ModelTask, ProtocolDescriptor, ProtocolExecutorKind, ProtocolScope, ProtocolTransportKind,
};
use nomifun_model_invoke::{
    ALL_MODEL_TASKS, AdapterRegistry, InvokeError, ProtocolAdapter, ProtocolManifestRegistry,
    RealtimeAdapterRegistry, RealtimeProtocolAdapter, RealtimeSession, RealtimeSessionConfig,
    ResolvedCall, ResolvedRealtimeCall, TaskOutcome, default_protocol_registry, platform_presets,
    preset_protocol_recommendation, protocol_manifest_for, protocol_manifest_for_connection,
    protocol_manifest_for_model_connection, protocol_task_descriptor, try_default_protocol_registry,
};

struct FakeRequestAdapter;

#[async_trait]
impl ProtocolAdapter for FakeRequestAdapter {
    fn id(&self) -> &'static str {
        "fake.request"
    }

    fn supports(&self, task: ModelTask) -> bool {
        task == ModelTask::Chat
    }

    async fn submit(
        &self,
        _http: &reqwest::Client,
        _call: &ResolvedCall,
    ) -> Result<TaskOutcome, InvokeError> {
        Err(InvokeError::config("unused fake adapter"))
    }
}

struct FakeRealtimeAdapter;

#[async_trait]
impl RealtimeProtocolAdapter for FakeRealtimeAdapter {
    fn id(&self) -> &'static str {
        "fake.realtime"
    }

    async fn open(
        &self,
        _call: &ResolvedRealtimeCall,
        _config: RealtimeSessionConfig,
    ) -> Result<RealtimeSession, InvokeError> {
        Err(InvokeError::config("unused fake realtime adapter"))
    }
}

fn fake_descriptor(id: &str) -> ProtocolDescriptor {
    ProtocolDescriptor {
        protocol_id: id.to_owned(),
        supported_tasks: vec![ModelTask::Chat],
        executor: ProtocolExecutorKind::Agent,
        transport: ProtocolTransportKind::Http,
        requires_output_ceiling: false,
        allowed_auth_schemes: vec!["bearer".to_owned()],
        scopes: vec![ProtocolScope::Custom],
        platforms: vec![],
        default_connections: vec![],
        endpoints: vec![],
        root_shape: None,
    }
}

#[test]
fn public_registry_is_enumerable_consistent_and_duplicate_safe() {
    let registry = try_default_protocol_registry().expect("default registry consistency");
    assert_eq!(registry.descriptors().count(), registry.len());
    assert!(registry.get("stepfun.images").is_some());
    assert!(registry.get("stepfun.realtime_s2s").is_some());
    assert!(registry.get("anthropic.messages").is_some());
    assert!(registry.get("bedrock.anthropic_messages").is_some());
    assert!(registry.get("gemini.generate_text").is_some());
    assert_eq!(
        registry.get("openai.chat_text").unwrap().allowed_auth_schemes,
        ["bearer"]
    );
    assert_eq!(
        registry.get("anthropic.messages").unwrap().allowed_auth_schemes,
        ["header_key:x-api-key"]
    );
    assert_eq!(
        registry.get("gemini.generate_text").unwrap().allowed_auth_schemes,
        ["header_key:x-goog-api-key"]
    );
    assert_eq!(
        registry
            .get("bedrock.anthropic_messages")
            .unwrap()
            .allowed_auth_schemes,
        ["bedrock"]
    );

    let error = ProtocolManifestRegistry::try_new(vec![
        fake_descriptor("duplicate"),
        fake_descriptor("duplicate"),
    ])
    .unwrap_err();
    assert!(error.message.contains("duplicate protocol descriptor"));
}

#[test]
fn execution_registries_reject_duplicate_ids_and_enumerate_stably() {
    let request_error = AdapterRegistry::try_new(vec![
        Arc::new(FakeRequestAdapter),
        Arc::new(FakeRequestAdapter),
    ])
    .err()
    .expect("duplicate request id");
    assert!(request_error.message.contains("fake.request"));
    let request = AdapterRegistry::new(vec![Arc::new(FakeRequestAdapter)]);
    assert_eq!(request.protocol_ids().collect::<Vec<_>>(), vec!["fake.request"]);

    let realtime_error = RealtimeAdapterRegistry::try_new(vec![
        Arc::new(FakeRealtimeAdapter),
        Arc::new(FakeRealtimeAdapter),
    ])
    .err()
    .expect("duplicate realtime id");
    assert!(realtime_error.message.contains("fake.realtime"));
    let realtime = RealtimeAdapterRegistry::new(vec![Arc::new(FakeRealtimeAdapter)]);
    assert_eq!(realtime.protocol_ids().collect::<Vec<_>>(), vec!["fake.realtime"]);
}

#[test]
fn manifest_prioritizes_preset_protocols_while_custom_without_a_model_stays_explicit() {
    let stepfun = protocol_manifest_for("StepFun", ModelTask::RealtimeConversation);
    assert_eq!(stepfun.tasks, ALL_MODEL_TASKS);
    assert_eq!(stepfun.protocols.len(), 1);
    assert_eq!(stepfun.protocols[0].protocol_id, "stepfun.realtime_s2s");
    assert_eq!(stepfun.recommendation.unwrap().protocol_id, "stepfun.realtime_s2s");

    let deepgram = protocol_manifest_for("Deepgram", ModelTask::RealtimeConversation);
    assert_eq!(deepgram.protocols.len(), 1);
    assert_eq!(deepgram.protocols[0].protocol_id, "stepfun.realtime_s2s");
    assert!(deepgram.protocols[0].default_connections.is_empty());
    assert!(deepgram.recommendation.is_none());

    let custom = protocol_manifest_for("custom", ModelTask::Chat);
    assert!(custom.recommendation.is_none());
    assert_eq!(
        custom
            .protocols
            .iter()
            .map(|descriptor| descriptor.protocol_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "anthropic.messages",
            "gemini.generate_text",
            "openai.chat_text"
        ]
    );
}

#[test]
fn custom_model_hint_recommends_the_unique_registry_declared_compat_protocol_per_task() {
    let registry = default_protocol_registry();
    for (task, expected_protocol) in [
        (ModelTask::Chat, "openai.chat_text"),
        (ModelTask::ImageGeneration, "openai.images"),
        (ModelTask::ImageEdit, "openai.images"),
        (ModelTask::VideoGeneration, "openai.videos"),
        (ModelTask::SpeechSynthesis, "openai.audio_speech"),
        (ModelTask::SpeechRecognition, "openai.audio_transcriptions"),
        (ModelTask::Embedding, "openai.embeddings"),
        (ModelTask::Rerank, "generic.rerank"),
    ] {
        let candidates = registry
            .descriptors()
            .filter(|descriptor| {
                descriptor.supported_tasks.contains(&task)
                    && descriptor.scopes.contains(&ProtocolScope::OfficialCompat)
                    && descriptor.scopes.contains(&ProtocolScope::Custom)
            })
            .collect::<Vec<_>>();
        assert_eq!(candidates.len(), 1, "{task:?} generic compatibility candidates");
        assert_eq!(candidates[0].protocol_id, expected_protocol);

        let view = protocol_manifest_for_model_connection(
            "custom",
            Some("https://gateway.example/v1"),
            Some("user-entered-model"),
            task,
        );
        let recommendation = view.recommendation.expect("custom model recommendation");
        assert_eq!(recommendation.protocol_id, expected_protocol);
        assert_eq!(recommendation.connection_role, None);
        assert_eq!(recommendation.default_base_url, None);
        assert_eq!(recommendation.default_auth_scheme.as_deref(), Some("bearer"));
        assert!(!recommendation.base_url_override_required);

        let descriptor = registry
            .get(&recommendation.protocol_id)
            .expect("recommended protocol is registered");
        assert!(descriptor.supported_tasks.contains(&task));
        assert!(descriptor.scopes.contains(&ProtocolScope::OfficialCompat));
        assert!(descriptor.scopes.contains(&ProtocolScope::Custom));
        assert!(
            view.protocols
                .iter()
                .find(|candidate| candidate.protocol_id == recommendation.protocol_id)
                .is_some_and(|candidate| candidate.default_connections.is_empty()),
            "custom recommendation must not expose a provider connection for {task:?}"
        );
    }
}

#[test]
fn custom_recommendation_requires_a_non_blank_model_and_has_no_realtime_default() {
    for model in [None, Some(""), Some("  \n") ] {
        let view = protocol_manifest_for_model_connection("custom", None, model, ModelTask::Chat);
        assert!(view.recommendation.is_none(), "blank model hint {model:?}");
    }

    let realtime = protocol_manifest_for_model_connection(
        "custom",
        None,
        Some("realtime-model"),
        ModelTask::RealtimeConversation,
    );
    assert!(realtime.recommendation.is_none());
    assert!(preset_protocol_recommendation("custom", ModelTask::Chat).is_none());
}

#[test]
fn model_hint_does_not_change_new_api_or_built_in_platform_recommendations() {
    let new_api = protocol_manifest_for_model_connection(
        "new-api",
        Some("https://gateway.example/v1"),
        Some("gpt-compatible-model"),
        ModelTask::Chat,
    );
    assert!(new_api.recommendation.is_none());

    for preset in platform_presets()
        .into_iter()
        .filter(|preset| preset.platform != "custom")
    {
        for task in ALL_MODEL_TASKS {
            let without_model =
                protocol_manifest_for_connection(&preset.preset, None, task);
            let with_model = protocol_manifest_for_model_connection(
                &preset.preset,
                None,
                Some("arbitrary-model-hint"),
                task,
            );
            assert_eq!(with_model, without_model, "{} {task:?}", preset.preset);
        }
    }
}

#[test]
fn unsupported_preset_tasks_expose_custom_protocols_without_endpoint_or_auth_defaults() {
    for (preset, task, expected_protocol) in [
        ("Deepgram", ModelTask::ImageGeneration, "openai.images"),
        ("StepFun", ModelTask::Embedding, "openai.embeddings"),
    ] {
        let view = protocol_manifest_for(preset, task);
        assert!(view.recommendation.is_none(), "{preset} {task:?}");
        assert!(!view.protocols.is_empty(), "{preset} {task:?}");
        assert!(
            view.protocols
                .iter()
                .any(|descriptor| descriptor.protocol_id == expected_protocol),
            "{preset} {task:?} must expose {expected_protocol}"
        );
        assert!(view.protocols.iter().all(|descriptor| {
            descriptor.scopes.contains(&ProtocolScope::Custom)
                && descriptor.default_connections.is_empty()
        }));
    }
}

#[test]
fn gemini_native_chat_and_regional_base_selection_are_exact() {
    let gemini = protocol_manifest_for("gemini", ModelTask::Chat);
    assert_eq!(gemini.protocols.len(), 3);
    assert_eq!(gemini.protocols[0].protocol_id, "gemini.generate_text");
    assert!(gemini.protocols[1..]
        .iter()
        .all(|descriptor| descriptor.default_connections.is_empty()));
    let recommendation = gemini.recommendation.unwrap();
    assert_eq!(recommendation.default_auth_scheme.as_deref(), Some("header_key:x-goog-api-key"));
    assert!(!recommendation.base_url_override_required);

    let cn = protocol_manifest_for_connection(
        "siliconflow",
        Some("https://api.siliconflow.cn/v1/"),
        ModelTask::Chat,
    );
    assert_eq!(cn.preset, "SiliconFlow-CN");
    assert_eq!(cn.platform_default_base_url.as_deref(), Some("https://api.siliconflow.cn/v1"));
    assert!(cn.protocols.iter().all(|descriptor| {
        descriptor
            .default_connections
            .iter()
            .all(|connection| connection.preset == "SiliconFlow-CN")
    }));
}

#[test]
fn all_presets_have_connection_defaults_or_explicit_user_input() {
    let presets = platform_presets();
    let ids = presets.iter().map(|preset| preset.preset.as_str()).collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), presets.len());
    for preset in presets {
        assert!(
            preset.platform_default_base_url.is_some() || preset.requires_user_input,
            "{}",
            preset.preset
        );
        assert!(preset.default_auth_scheme.is_some(), "{}", preset.preset);
    }
}

#[test]
fn backend_catalog_covers_every_ui_model_platform_preset() {
    let source = include_str!("../../../../ui/src/renderer/utils/model/modelPlatforms.ts");
    let section = source
        .split("export const MODEL_PLATFORMS")
        .nth(1)
        .expect("MODEL_PLATFORMS declaration")
        .split("export const NEW_API_PROTOCOL_OPTIONS")
        .next()
        .expect("MODEL_PLATFORMS section");
    let mut ui_ids = BTreeSet::new();
    for line in section.lines() {
        let Some((_, after)) = line.split_once("value:") else {
            continue;
        };
        let after = after.trim_start();
        let Some(quote) = after.chars().next().filter(|value| matches!(value, '\'' | '"')) else {
            continue;
        };
        let tail = &after[quote.len_utf8()..];
        let Some(end) = tail.find(quote) else {
            continue;
        };
        ui_ids.insert(tail[..end].to_owned());
    }
    let backend_ids = platform_presets()
        .into_iter()
        .map(|preset| preset.preset)
        .collect::<BTreeSet<_>>();
    assert_eq!(backend_ids, ui_ids);
}

#[test]
fn all_recommended_lifecycle_urls_match_the_locked_official_matrix() {
    let registry = default_protocol_registry();
    let mut lines = Vec::new();
    for preset in platform_presets() {
        for task in ALL_MODEL_TASKS {
            let view = protocol_manifest_for(&preset.preset, task);
            let Some(recommendation) = &view.recommendation else {
                continue;
            };
            let descriptor = protocol_task_descriptor(&recommendation.protocol_id, task)
                .expect("recommended descriptor");
            if descriptor.transport == ProtocolTransportKind::Sdk {
                lines.push(format!(
                    "{}|{:?}|{}|sdk",
                    preset.preset, task, recommendation.protocol_id
                ));
                continue;
            }
            let base = recommendation.default_base_url.as_deref().expect("network base URL");
            for endpoint in descriptor.endpoints {
                // Compose through the production joiner so this snapshot cannot
                // stay green while the real URL assembly regresses.
                let mut url = nomifun_model_invoke::join_endpoint(base, &endpoint.default_value);
                if descriptor.transport == ProtocolTransportKind::Websocket {
                    if let Some(tail) = url.strip_prefix("https://") {
                        url = format!("wss://{tail}");
                    } else if let Some(tail) = url.strip_prefix("http://") {
                        url = format!("ws://{tail}");
                    }
                }
                for duplicated in [
                    "/v1/v1/",
                    "/api/v3/api/v3/",
                    "/api/paas/v4/api/paas/v4/",
                    "/compatible-mode/v1/api/v1/",
                ] {
                    assert!(!url.contains(duplicated), "bad composed URL {url}");
                }
                lines.push(format!(
                    "{}|{:?}|{}|{:?}|{}|{}",
                    preset.preset,
                    task,
                    recommendation.protocol_id,
                    endpoint.purpose,
                    endpoint.field,
                    url
                ));
            }
        }
    }
    assert!(!registry.is_empty());
    let snapshot = lines.join("\n");
    let hash = snapshot.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    assert_eq!(
        hash, 9_446_312_405_170_401_367,
        "recommendation URL snapshot changed:\n{snapshot}"
    );
}
