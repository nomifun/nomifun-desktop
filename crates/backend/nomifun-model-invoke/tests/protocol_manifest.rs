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
    protocol_manifest_for_model_connection, try_default_protocol_registry,
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
fn manifest_prioritizes_preset_protocols_and_installs_a_custom_compat_default() {
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
    assert_eq!(
        custom.recommendation.as_ref().unwrap().protocol_id,
        "openai.chat_text"
    );
    assert_eq!(
        custom
            .protocols
            .iter()
            .map(|descriptor| descriptor.protocol_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "anthropic.messages",
            "gemini.generate_text",
            "openai.chat_text",
            "openai.responses"
        ]
    );
}

#[test]
fn custom_presets_default_to_chat_and_keep_other_task_templates() {
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
        if task != ModelTask::Chat { assert_eq!(candidates[0].protocol_id, expected_protocol); }

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
        if task != ModelTask::Chat { assert!(descriptor.scopes.contains(&ProtocolScope::OfficialCompat)); }
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
fn openai_prefers_responses_and_compatible_providers_can_select_it_explicitly() {
    let openai = protocol_manifest_for("OpenAI", ModelTask::Chat);
    let recommendation = openai.recommendation.unwrap();
    assert_eq!(recommendation.protocol_id, "openai.responses");
    assert_eq!(recommendation.default_base_url.as_deref(), Some("https://api.openai.com/v1"));
    assert!(openai.protocols.iter().any(|descriptor| descriptor.protocol_id == "openai.chat_text"));

    for (platform, expected) in [("custom","openai.chat_text"),("new-api","openai.chat_text"),
        ("openrouter","openai.chat_text"),("deepseek","openai.chat_text")] {
        let manifest = protocol_manifest_for(platform, ModelTask::Chat);
        assert_eq!(manifest.recommendation.unwrap().protocol_id, expected, "{platform}");
        let responses = manifest.protocols.iter().find(|descriptor| descriptor.protocol_id == "openai.responses")
            .expect("Responses is an explicitly selectable serializer");
        assert!(responses.scopes.contains(&ProtocolScope::Custom));
        assert!(responses.default_connections.iter().all(|connection| !connection.base_url.contains("api.openai.com")),
            "do not import OpenAI URLs into {platform}");
    }
}

#[test]
fn custom_recommendation_needs_no_model_hint_and_has_no_realtime_default() {
    for model in [None, Some(""), Some("  \n") ] {
        let view = protocol_manifest_for_model_connection("custom", None, model, ModelTask::Chat);
        assert_eq!(
            view.recommendation.as_ref().map(|value| value.protocol_id.as_str()),
            Some("openai.chat_text"),
            "blank model hint {model:?}"
        );
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
fn new_api_keeps_generic_defaults_and_unknown_media_models_get_no_guessed_route() {
    let new_api = protocol_manifest_for_model_connection(
        "new-api",
        Some("https://gateway.example/v1"),
        Some("gpt-compatible-model"),
        ModelTask::Chat,
    );
    assert_eq!(
        new_api.recommendation.as_ref().map(|value| value.protocol_id.as_str()),
        Some("openai.chat_text")
    );

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
            if task == ModelTask::Chat {
                assert_eq!(with_model.protocols,without_model.protocols);
                assert!(with_model.model_defaults.is_none());
            } else if !matches!(preset.platform.as_str(),"new-api" | "nomifun-model-gateway") {
                assert!(with_model.recommendation.is_none(),"{} {task:?} unknown model",preset.preset);
                assert_eq!(with_model.protocols,without_model.protocols);
            }
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
    assert_eq!(gemini.protocols.len(), 4);
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
            let descriptor = view.protocols.iter().find(|protocol| protocol.protocol_id == recommendation.protocol_id)
                .expect("recommended configuration descriptor");
            if descriptor.transport == ProtocolTransportKind::Sdk {
                lines.push(format!(
                    "{}|{:?}|{}|sdk",
                    preset.preset, task, recommendation.protocol_id
                ));
                continue;
            }
            let Some(base) = recommendation.default_base_url.as_deref() else {
                assert!(
                    view.requires_user_input
                        && matches!(view.platform.as_str(), "custom" | "new-api"),
                    "{} {:?} recommended a network protocol without a URL",
                    preset.preset,
                    task
                );
                // Compatibility defaults intentionally inherit the user-entered
                // provider root, so there is no official lifecycle URL to lock.
                continue;
            };
            for endpoint in descriptor.endpoints.iter().filter(|endpoint| endpoint.task == task) {
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
        hash, 15_772_525_078_110_188_208,
        "recommendation URL snapshot changed:\n{snapshot}"
    );
}

#[test]
fn compatible_media_routes_use_the_documented_vendor_path() {
    for (platform,model,task,protocol,path) in [
        ("openrouter","openai/gpt-image-2",ModelTask::ImageGeneration,"openai.images","/images"),
        ("ctyun","qwen3-rerank",ModelTask::Rerank,"generic.rerank","/reranks"),
    ] {
        let view = protocol_manifest_for_model_connection(platform,None,Some(model),task);
        let descriptor = view.protocols.iter().find(|descriptor| descriptor.protocol_id == protocol).unwrap();
        assert_eq!(descriptor.endpoints.iter().find(|endpoint| endpoint.task == task && endpoint.field == "endpoint").unwrap().default_value,path);
    }
}

#[test]
fn dashscope_native_roots_keep_the_configured_official_region() {
    for origin in [
        "https://dashscope.aliyuncs.com",
        "https://dashscope-intl.aliyuncs.com",
        "https://dashscope-us.aliyuncs.com",
        "https://cn-hongkong.dashscope.aliyuncs.com",
    ] {
        for (task, model, protocol) in [
            (ModelTask::ImageGeneration, "qwen-image-3.0-pro", "dashscope.images"),
            (ModelTask::Embedding, "text-embedding-v4", "dashscope.embeddings"),
        ] {
            let chat_root = format!("{origin}/compatible-mode/v1/");
            let manifest = protocol_manifest_for_model_connection("dashscope", Some(&chat_root), Some(model), task);
            let recommendation = manifest.recommendation.expect("documented native model");
            assert_eq!(recommendation.protocol_id, protocol);
            assert_eq!(recommendation.default_base_url.as_deref(), Some(origin));
            assert!(recommendation.base_url_override_required);
            let descriptor = manifest.protocols.iter().find(|entry| entry.protocol_id == protocol).unwrap();
            assert!(nomifun_model_invoke::url_algebra::root_matches_shape(origin, descriptor.root_shape.unwrap()));
            for endpoint in descriptor.endpoints.iter().filter(|entry| entry.task == task) {
                let url = nomifun_model_invoke::join_endpoint(origin, &endpoint.default_value);
                assert!(url.starts_with(&format!("{origin}/api/v1/")), "{url}");
                assert!(!url.contains("/compatible-mode/"), "{url}");
            }
        }
    }
    let domestic = protocol_manifest_for_model_connection(
        "dashscope", None, Some("text-embedding-v4"), ModelTask::Embedding,
    );
    assert_eq!(domestic.recommendation.unwrap().default_base_url.as_deref(), Some("https://dashscope.aliyuncs.com"));
}

#[test]
fn dashscope_native_recommendations_leave_relays_and_unknown_workspace_roots_intact() {
    for root in [
        "https://relay.example/compatible-mode/v1",
        "https://dashscope-intl.aliyuncs.com.relay.example/v1",
        "https://unknown-workspace.ap-southeast-1.maas.aliyuncs.com/compatible-mode/v1",
        "https://dashscope-intl.aliyuncs.com:8443/compatible-mode/v1",
    ] {
        let manifest = protocol_manifest_for_model_connection(
            "dashscope", Some(root), Some("text-embedding-v4"), ModelTask::Embedding,
        );
        let recommendation = manifest.recommendation.unwrap();
        assert_eq!(recommendation.default_base_url.as_deref(), Some(root));
        assert!(!recommendation.base_url_override_required);
    }
    let chat_root = "https://dashscope-intl.aliyuncs.com/compatible-mode/v1";
    let chat = protocol_manifest_for_model_connection("dashscope", Some(chat_root), Some("qwen3.8-max"), ModelTask::Chat);
    assert_eq!(chat.recommendation.unwrap().default_base_url.as_deref(), Some(chat_root));
}

#[test]
fn chat_defaults_do_not_inherit_responses_support_as_a_preference() {
    for (platform,model) in [("stepfun","step-5-preview"),("stepfun","step-3.7-flash"),
        ("openrouter","vendor/model"),("ppio","new-model"),("poe","bot"),
        ("modelscope","Qwen/Qwen3.5-397B-A17B"),("agnes","agnes-2.5-flash"),
        ("zhipu","glm-5.3"),("glm-coding-plan","glm-5.3"),("dashscope","qwen3.7-plus"),
        ("hunyuan","hy4-preview"),("ark-agent-plan","current-text-model")] {
        let view=protocol_manifest_for_model_connection(platform,None,Some(model),ModelTask::Chat);
        assert_eq!(view.recommendation.unwrap().protocol_id,"openai.chat_text","{platform}/{model}");
        assert!(view.protocols.iter().any(|protocol| protocol.protocol_id == "openai.responses"),"Responses stays configurable for {platform}");
    }
    for (platform,model) in [("openai","gpt-6-astra"),("minimax","MiniMax-M3"),
        ("minimax-coding-plan","MiniMax-M3.1-Flash-Preview"),("xai","grok-4.7")] {
        assert_eq!(protocol_manifest_for_model_connection(platform,None,Some(model),ModelTask::Chat)
            .recommendation.unwrap().protocol_id,"openai.responses","documented exception {platform}");
    }
    let gateway=protocol_manifest_for_model_connection("nomifun-model-gateway",Some("https://gateway.example/v1"),Some("catalog-model"),ModelTask::Chat);
    assert!(gateway.recommendation.is_none(),"gateway catalog owns its exact protocol");
}
