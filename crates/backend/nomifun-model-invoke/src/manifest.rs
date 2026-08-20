//! Enumerable protocol and provider-preset metadata.
//!
//! Runtime dispatch selects an explicitly persisted protocol. This module is
//! the configuration authority exposed to clients; its preset recommendations
//! are never a runtime fallback.

use std::collections::{BTreeMap, BTreeSet};

use nomifun_api_types::ModelTask;
pub use nomifun_api_types::{
    AuthSchemeDescriptor, EndpointRootShape, ModelProtocolManifestResponse,
    PlatformPresetDescriptor, ProtocolDefaultConnection, ProtocolDescriptor,
    ProtocolEndpointDescriptor, ProtocolEndpointPurpose, ProtocolExecutorKind,
    ProtocolRecommendation, ProtocolScope, ProtocolTaskDescriptor, ProtocolTransportKind,
};

use crate::adapter::AdapterRegistry;
use crate::adapters::{
    default_adapters, default_realtime_adapters, is_reserved_local_transport_param_key,
};
use crate::error::InvokeError;
use crate::realtime::RealtimeAdapterRegistry;
use crate::routes_table::preset_protocol_recommendation;

pub const ALL_MODEL_TASKS: [ModelTask; 9] = [
    ModelTask::Chat,
    ModelTask::RealtimeConversation,
    ModelTask::ImageGeneration,
    ModelTask::ImageEdit,
    ModelTask::VideoGeneration,
    ModelTask::SpeechSynthesis,
    ModelTask::SpeechRecognition,
    ModelTask::Embedding,
    ModelTask::Rerank,
];

#[derive(Debug, Clone)]
pub struct ProtocolManifestRegistry {
    by_id: BTreeMap<String, ProtocolDescriptor>,
}

impl ProtocolManifestRegistry {
    pub fn try_new(descriptors: Vec<ProtocolDescriptor>) -> Result<Self, InvokeError> {
        let mut by_id = BTreeMap::new();
        for descriptor in descriptors {
            let id = descriptor.protocol_id.trim().to_owned();
            if id.is_empty() {
                return Err(InvokeError::config("protocol descriptor id cannot be empty"));
            }
            if descriptor.supported_tasks.is_empty() {
                return Err(InvokeError::config(format!(
                    "protocol descriptor {id:?} has no supported tasks"
                )));
            }
            if descriptor.allowed_auth_schemes.is_empty() {
                return Err(InvokeError::config(format!(
                    "protocol descriptor {id:?} has no allowed auth schemes"
                )));
            }
            let unique_auth_schemes = descriptor
                .allowed_auth_schemes
                .iter()
                .map(|scheme| scheme.trim().to_ascii_lowercase())
                .collect::<BTreeSet<_>>();
            if unique_auth_schemes.contains("")
                || unique_auth_schemes.len() != descriptor.allowed_auth_schemes.len()
            {
                return Err(InvokeError::config(format!(
                    "protocol descriptor {id:?} contains a blank or duplicate auth scheme"
                )));
            }
            let unique_tasks = descriptor
                .supported_tasks
                .iter()
                .copied()
                .map(TaskOrdinal::from)
                .collect::<BTreeSet<_>>();
            if unique_tasks.len() != descriptor.supported_tasks.len() {
                return Err(InvokeError::config(format!(
                    "protocol descriptor {id:?} contains a duplicate task"
                )));
            }
            let mut endpoint_keys = BTreeSet::new();
            for endpoint in &descriptor.endpoints {
                if !descriptor.supported_tasks.contains(&endpoint.task) {
                    return Err(InvokeError::config(format!(
                        "protocol descriptor {id:?} has an endpoint for unsupported task {:?}",
                        endpoint.task
                    )));
                }
                let key = (
                    TaskOrdinal::from(endpoint.task),
                    endpoint.field.clone(),
                    endpoint.purpose,
                );
                if !endpoint_keys.insert(key) {
                    return Err(InvokeError::config(format!(
                        "protocol descriptor {id:?} contains a duplicate endpoint for task {:?}, field {:?}, purpose {:?}",
                        endpoint.task, endpoint.field, endpoint.purpose
                    )));
                }
                let allowed = endpoint
                    .allowed_placeholders
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>();
                if allowed.len() != endpoint.allowed_placeholders.len()
                    || allowed.iter().any(|name| !valid_placeholder_name(name))
                {
                    return Err(InvokeError::config(format!(
                        "protocol descriptor {id:?} has a blank, malformed or duplicate placeholder for field {:?}",
                        endpoint.field
                    )));
                }
                let required = endpoint
                    .required_placeholders
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>();
                if required.len() != endpoint.required_placeholders.len()
                    || !required.is_subset(&allowed)
                {
                    return Err(InvokeError::config(format!(
                        "protocol descriptor {id:?} has an invalid required placeholder contract for field {:?}",
                        endpoint.field
                    )));
                }
                let defaults = collect_endpoint_placeholders(&endpoint.default_value)?;
                if !defaults.is_subset(&allowed)
                    || (!required.is_empty() && defaults.is_disjoint(&required))
                {
                    return Err(InvokeError::config(format!(
                        "protocol descriptor {id:?} default endpoint {:?} does not satisfy its placeholder contract",
                        endpoint.field
                    )));
                }
                // The declared version convention must match the template it
                // describes. Without this, `root_shape` could silently disagree
                // with `default_value` and the UI would state the wrong rule —
                // which is exactly the class of defect this field exists to end.
                let template_is_versioned = endpoint
                    .default_value
                    .split(['?', '#'])
                    .next()
                    .unwrap_or_default()
                    .split('/')
                    .any(crate::url_algebra::is_version_segment);
                let declared_origin_root =
                    endpoint.root_shape == nomifun_api_types::EndpointRootShape::OriginRoot;
                if template_is_versioned != declared_origin_root {
                    return Err(InvokeError::config(format!(
                        "protocol descriptor {id:?} endpoint {:?} declares {:?} but its template {:?} says otherwise",
                        endpoint.field, endpoint.root_shape, endpoint.default_value
                    )));
                }
                if endpoint.root_shape != descriptor.endpoints[0].root_shape {
                    return Err(InvokeError::config(format!(
                        "protocol descriptor {id:?} mixes endpoint root shapes; one protocol has one convention"
                    )));
                }
            }
            // A shipped default connection must satisfy its own protocol's
            // convention, or the preset would hand the user a root that cannot
            // work with the endpoint template it is paired with.
            if let Some(shape) = descriptor.endpoints.first().map(|first| first.root_shape) {
                for connection in &descriptor.default_connections {
                    if !crate::url_algebra::root_matches_shape(&connection.base_url, shape) {
                        return Err(InvokeError::config(format!(
                            "protocol descriptor {id:?} default connection for preset {:?} has base_url {:?}, which contradicts {shape:?}",
                            connection.preset, connection.base_url
                        )));
                    }
                }
            }
            if by_id.insert(id.clone(), descriptor).is_some() {
                return Err(InvokeError::config(format!(
                    "duplicate protocol descriptor id {id:?}"
                )));
            }
        }
        Ok(Self { by_id })
    }

    pub fn descriptors(&self) -> impl ExactSizeIterator<Item = &ProtocolDescriptor> {
        self.by_id.values()
    }

    pub fn get(&self, protocol_id: &str) -> Option<&ProtocolDescriptor> {
        self.by_id.get(protocol_id)
    }

    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

// ModelTask intentionally does not expose ordering; this wrapper gives us a
// deterministic set solely for duplicate validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TaskOrdinal(u8);

impl From<ModelTask> for TaskOrdinal {
    fn from(task: ModelTask) -> Self {
        Self(match task {
            ModelTask::Chat => 0,
            ModelTask::RealtimeConversation => 1,
            ModelTask::ImageGeneration => 2,
            ModelTask::ImageEdit => 3,
            ModelTask::VideoGeneration => 4,
            ModelTask::SpeechSynthesis => 5,
            ModelTask::SpeechRecognition => 6,
            ModelTask::Embedding => 7,
            ModelTask::Rerank => 8,
        })
    }
}

#[derive(Clone, Copy)]
struct PresetSpec {
    preset: &'static str,
    platform: &'static str,
    base_url: Option<&'static str>,
    requires_user_input: bool,
    auth_scheme: Option<&'static str>,
}

const fn preset(
    preset: &'static str,
    platform: &'static str,
    base_url: &'static str,
) -> PresetSpec {
    PresetSpec {
        preset,
        platform,
        base_url: Some(base_url),
        requires_user_input: false,
        auth_scheme: Some("bearer"),
    }
}

const PRESETS: &[PresetSpec] = &[
    PresetSpec { preset: "custom", platform: "custom", base_url: None, requires_user_input: true, auth_scheme: Some("bearer") },
    PresetSpec { preset: "new-api", platform: "new-api", base_url: None, requires_user_input: true, auth_scheme: Some("bearer") },
    PresetSpec { preset: "gemini", platform: "gemini", base_url: Some("https://generativelanguage.googleapis.com"), requires_user_input: false, auth_scheme: Some("header_key:x-goog-api-key") },
    preset("OpenAI", "openai", "https://api.openai.com/v1"),
    PresetSpec { preset: "Anthropic", platform: "anthropic", base_url: Some("https://api.anthropic.com"), requires_user_input: false, auth_scheme: Some("header_key:x-api-key") },
    PresetSpec { preset: "AWS-Bedrock", platform: "bedrock", base_url: None, requires_user_input: true, auth_scheme: Some("bedrock") },
    preset("DeepSeek", "deepseek", "https://api.deepseek.com/v1"),
    PresetSpec { preset: "Deepgram", platform: "deepgram", base_url: Some("https://api.deepgram.com"), requires_user_input: false, auth_scheme: Some("token") },
    preset("MiMo", "mimo", "https://api.xiaomimimo.com/v1"),
    preset("MiMo-Token-Plan-CN", "mimo-token-plan-cn", "https://token-plan-cn.xiaomimimo.com/v1"),
    preset("MiMo-Token-Plan-SGP", "mimo-token-plan-sgp", "https://token-plan-sgp.xiaomimimo.com/v1"),
    preset("MiMo-Token-Plan-AMS", "mimo-token-plan-ams", "https://token-plan-ams.xiaomimimo.com/v1"),
    preset("MiniMax", "minimax", "https://api.minimaxi.com/v1"),
    preset("MiniMax-Code", "minimax-code", "https://api.minimax.io/v1"),
    preset("MiniMax-Coding-Plan", "minimax-coding-plan", "https://api.minimaxi.com/v1"),
    preset("Novita", "novita", "https://api.novita.ai/openai/v1"),
    preset("OpenRouter", "openrouter", "https://openrouter.ai/api/v1"),
    preset("Dashscope", "dashscope", "https://dashscope.aliyuncs.com/compatible-mode/v1"),
    preset("Dashscope-Coding", "dashscope-coding", "https://coding.dashscope.aliyuncs.com/v1"),
    preset("SiliconFlow-CN", "siliconflow", "https://api.siliconflow.cn/v1"),
    preset("SiliconFlow", "siliconflow", "https://api.siliconflow.com/v1"),
    preset("Zhipu", "zhipu", "https://open.bigmodel.cn/api/paas/v4"),
    preset("GLM-Coding-Plan", "glm-coding-plan", "https://open.bigmodel.cn/api/coding/paas/v4"),
    preset("Moonshot", "moonshot-cn", "https://api.moonshot.cn/v1"),
    preset("Moonshot-Global", "moonshot-global", "https://api.moonshot.ai/v1"),
    preset("xAI", "xai", "https://api.x.ai/v1"),
    preset("Ark", "ark", "https://ark.cn-beijing.volces.com/api/v3"),
    preset("Ark-Coding-Plan", "ark-coding-plan", "https://ark.cn-beijing.volces.com/api/coding/v3"),
    preset("Ark-Agent-Plan", "ark-agent-plan", "https://ark.cn-beijing.volces.com/api/plan/v3"),
    preset("Qianfan", "qianfan", "https://qianfan.baidubce.com/v2"),
    preset("Qianfan-Coding-Plan", "qianfan-coding-plan", "https://qianfan.baidubce.com/v2/coding"),
    preset("Hunyuan", "hunyuan", "https://tokenhub.tencentmaas.com/v1"),
    preset("Hunyuan-Global", "hunyuan-global", "https://tokenhub-intl.tencentmaas.com/v1"),
    preset("Lingyi", "lingyi", "https://api.lingyiwanwu.com/v1"),
    preset("Poe", "poe", "https://api.poe.com/v1"),
    preset("PPIO", "ppio", "https://api.ppio.com/openai/v1"),
    preset("ModelScope", "modelscope", "https://api-inference.modelscope.cn/v1"),
    preset("InfiniAI", "infiniai", "https://cloud.infini-ai.com/maas/v1"),
    preset("Ctyun", "ctyun", "https://ai.ctaigw.cn/v1"),
    preset("StepFun", "stepfun", "https://api.stepfun.com/v1"),
    preset("StepFun-Plan", "stepfun-plan", "https://api.stepfun.com/step_plan/v1"),
];

pub fn platform_presets() -> Vec<PlatformPresetDescriptor> {
    PRESETS.iter().copied().map(owned_preset).collect()
}

fn owned_preset(spec: PresetSpec) -> PlatformPresetDescriptor {
    PlatformPresetDescriptor {
        preset: spec.preset.to_owned(),
        platform: spec.platform.to_owned(),
        platform_default_base_url: spec.base_url.map(str::to_owned),
        requires_user_input: spec.requires_user_input,
        default_auth_scheme: spec.auth_scheme.map(str::to_owned),
    }
}

fn normalized_base_url(value: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(value.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https" | "ws" | "wss") || url.host_str().is_none() {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(if path.is_empty() { "/" } else { &path });
    Some(url.to_string().trim_end_matches('/').to_ascii_lowercase())
}

fn resolve_preset(value: &str, base_url_hint: Option<&str>) -> PlatformPresetDescriptor {
    let value = value.trim();
    let found = PRESETS
        .iter()
        .find(|entry| entry.preset == value)
        .or_else(|| {
            let wanted_base = normalized_base_url(base_url_hint?)?;
            PRESETS.iter().find(|entry| {
                (entry.platform == value || entry.preset.eq_ignore_ascii_case(value))
                    && entry
                        .base_url
                        .and_then(normalized_base_url)
                        .as_deref()
                        == Some(wanted_base.as_str())
            })
        })
        .or_else(|| PRESETS.iter().find(|entry| entry.preset.eq_ignore_ascii_case(value)))
        .or_else(|| PRESETS.iter().find(|entry| entry.platform == value));
    found.copied().map(owned_preset).unwrap_or_else(|| PlatformPresetDescriptor {
        preset: value.to_owned(),
        platform: value.to_ascii_lowercase(),
        platform_default_base_url: None,
        requires_user_input: true,
        default_auth_scheme: None,
    })
}

#[derive(Clone, Copy)]
struct EndpointSpec {
    task: ModelTask,
    field: &'static str,
    purpose: ProtocolEndpointPurpose,
    method: Option<&'static str>,
    default_value: &'static str,
    editable: bool,
    /// Which half of the URL owns the API version segment. Enforced against
    /// `default_value` by [`ProtocolManifestRegistry::try_new`], so a template
    /// and its declaration cannot drift apart.
    root: EndpointRootShape,
}

/// An endpoint whose template is version-free, so the connection root must
/// carry the version (`https://host/v1` + `/chat/completions`).
const fn endpoint(
    task: ModelTask,
    field: &'static str,
    purpose: ProtocolEndpointPurpose,
    method: &'static str,
    default_value: &'static str,
) -> EndpointSpec {
    EndpointSpec {
        task,
        field,
        purpose,
        method: Some(method),
        default_value,
        editable: true,
        root: EndpointRootShape::VersionedRoot,
    }
}

/// An endpoint whose template carries the version itself, so the connection
/// root must be version-free (`https://host` + `/v1/messages`).
const fn origin_endpoint(
    task: ModelTask,
    field: &'static str,
    purpose: ProtocolEndpointPurpose,
    method: &'static str,
    default_value: &'static str,
) -> EndpointSpec {
    EndpointSpec {
        task,
        field,
        purpose,
        method: Some(method),
        default_value,
        editable: true,
        root: EndpointRootShape::OriginRoot,
    }
}

#[derive(Clone, Copy)]
struct ProtocolSpec {
    id: &'static str,
    tasks: &'static [ModelTask],
    executor: ProtocolExecutorKind,
    transport: ProtocolTransportKind,
    scopes: &'static [ProtocolScope],
    platforms: &'static [&'static str],
    connection_role: Option<&'static str>,
    endpoints: &'static [EndpointSpec],
}

const ALL_SCOPES: &[ProtocolScope] = &[
    ProtocolScope::Native,
    ProtocolScope::OfficialCompat,
    ProtocolScope::Custom,
];
const NATIVE_CUSTOM: &[ProtocolScope] = &[ProtocolScope::Native, ProtocolScope::Custom];
const COMPAT_CUSTOM: &[ProtocolScope] = &[ProtocolScope::OfficialCompat, ProtocolScope::Custom];
const NATIVE_ONLY: &[ProtocolScope] = &[ProtocolScope::Native];

const GENERIC_HTTP_AUTH_SCHEMES: &[&str] = &[
    "bearer",
    "token",
    "header_key:<name>",
    "query_key:<param>",
];

fn allowed_auth_schemes(spec: ProtocolSpec) -> &'static [&'static str] {
    match spec.id {
        // Agent executors enforce these exact schemes before constructing the
        // provider client; advertising broader transport vocabulary would make
        // a model save successfully and fail on its first invocation.
        "openai.chat_text" | "openai.responses" => &["bearer"],
        "anthropic.messages" => &["header_key:x-api-key"],
        "gemini.generate_text" => &["header_key:x-goog-api-key"],
        "bedrock.anthropic_messages" => &["bedrock"],
        // The persistent StepFun session performs the same strict check before
        // sending its Bearer header over the WebSocket handshake.
        "stepfun.realtime_s2s" => &["bearer"],
        // Volcengine voice uses three required X-Api-* headers populated from
        // its purpose-built credential object.
        "volc.asr_file" | "volc.tts_v3" => &["volc_voice"],
        // One-shot HTTP adapters all delegate auth to the shared transport,
        // which supports these four single-key schemes. Keeping the
        // parameterized forms allows a future compatible provider to reuse an
        // existing serializer without NomiFun guessing a vendor default.
        _ => GENERIC_HTTP_AUTH_SCHEMES,
    }
}

/// Whether a Chat protocol's wire schema mandates an explicit output ceiling.
/// Kept exhaustive over the registered Anthropic-family Agent protocols and
/// pinned against the runtime ProviderType policy by nomifun-ai-agent tests.
pub fn protocol_requires_output_ceiling(protocol_id: &str) -> bool {
    matches!(
        protocol_id,
        "anthropic.messages" | "bedrock.anthropic_messages"
    )
}

const OPENAI_CHAT_PLATFORMS: &[&str] = &[
    "openai", "deepseek", "mimo", "mimo-token-plan-cn", "mimo-token-plan-sgp",
    "mimo-token-plan-ams", "minimax", "minimax-code", "minimax-coding-plan", "novita",
    "openrouter", "dashscope", "dashscope-coding", "siliconflow", "zhipu", "glm-coding-plan",
    "moonshot-cn", "moonshot-global", "xai", "ark", "ark-coding-plan", "ark-agent-plan",
    "qianfan", "qianfan-coding-plan", "hunyuan", "hunyuan-global", "lingyi", "poe", "ppio",
    "modelscope", "infiniai", "ctyun", "stepfun", "stepfun-plan",
];

use ModelTask::{
    Chat, Embedding, ImageEdit, ImageGeneration, RealtimeConversation, Rerank, SpeechRecognition,
    SpeechSynthesis, VideoGeneration,
};
use ProtocolEndpointPurpose::{Content, Poll, Session, Submit};
use ProtocolExecutorKind::{Agent, AsyncJob, ModelInvoke, RealtimeSession};
use ProtocolTransportKind::{Http, Sdk, Websocket};

const PROTOCOL_SPECS: &[ProtocolSpec] = &[
    ProtocolSpec { id: "openai.chat_text", tasks: &[Chat], executor: Agent, transport: Http, scopes: ALL_SCOPES, platforms: OPENAI_CHAT_PLATFORMS, connection_role: None, endpoints: &[endpoint(Chat, "endpoint", Submit, "POST", "/chat/completions")] },
    ProtocolSpec { id: "openai.responses", tasks: &[Chat], executor: Agent, transport: Http, scopes: NATIVE_ONLY, platforms: &["openai"], connection_role: None, endpoints: &[endpoint(Chat, "endpoint", Submit, "POST", "/responses")] },
    ProtocolSpec { id: "anthropic.messages", tasks: &[Chat], executor: Agent, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["anthropic"], connection_role: None, endpoints: &[origin_endpoint(Chat, "endpoint", Submit, "POST", "/v1/messages")] },
    ProtocolSpec { id: "bedrock.anthropic_messages", tasks: &[Chat], executor: Agent, transport: Sdk, scopes: NATIVE_ONLY, platforms: &["bedrock"], connection_role: None, endpoints: &[] },
    ProtocolSpec { id: "gemini.generate_text", tasks: &[Chat], executor: Agent, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["gemini"], connection_role: None, endpoints: &[origin_endpoint(Chat, "endpoint", Submit, "POST", "/v1beta/models/{model}:streamGenerateContent?alt=sse")] },
    ProtocolSpec { id: "openai.images", tasks: &[ImageGeneration, ImageEdit], executor: ModelInvoke, transport: Http, scopes: ALL_SCOPES, platforms: &["openai", "ctyun"], connection_role: None, endpoints: &[
        endpoint(ImageGeneration, "endpoint", Submit, "POST", "/images/generations"),
        endpoint(ImageEdit, "endpoint", Submit, "POST", "/images/edits"),
    ] },
    ProtocolSpec { id: "openai.videos", tasks: &[VideoGeneration], executor: AsyncJob, transport: Http, scopes: ALL_SCOPES, platforms: &["openai"], connection_role: None, endpoints: &[
        endpoint(VideoGeneration, "endpoint", Submit, "POST", "/videos"),
        endpoint(VideoGeneration, "poll_endpoint", Poll, "GET", "/videos/{id}"),
        endpoint(VideoGeneration, "content_endpoint", Content, "GET", "/videos/{id}/content"),
    ] },
    ProtocolSpec { id: "openai.embeddings", tasks: &[Embedding], executor: ModelInvoke, transport: Http, scopes: ALL_SCOPES, platforms: &["openai", "novita", "openrouter", "siliconflow", "ppio", "infiniai", "qianfan", "hunyuan", "hunyuan-global", "ctyun", "zhipu"], connection_role: None, endpoints: &[endpoint(Embedding, "endpoint", Submit, "POST", "/embeddings")] },
    ProtocolSpec { id: "generic.rerank", tasks: &[Rerank], executor: ModelInvoke, transport: Http, scopes: COMPAT_CUSTOM, platforms: &["siliconflow", "ppio", "qianfan", "ctyun", "zhipu"], connection_role: None, endpoints: &[endpoint(Rerank, "endpoint", Submit, "POST", "/rerank")] },
    ProtocolSpec { id: "openai.audio_transcriptions", tasks: &[SpeechRecognition], executor: ModelInvoke, transport: Http, scopes: ALL_SCOPES, platforms: &["openai", "siliconflow"], connection_role: None, endpoints: &[endpoint(SpeechRecognition, "endpoint", Submit, "POST", "/audio/transcriptions")] },
    ProtocolSpec { id: "openai.audio_speech", tasks: &[SpeechSynthesis], executor: ModelInvoke, transport: Http, scopes: ALL_SCOPES, platforms: &["openai"], connection_role: None, endpoints: &[endpoint(SpeechSynthesis, "endpoint", Submit, "POST", "/audio/speech")] },
    ProtocolSpec { id: "gemini.generate_content", tasks: &[ImageGeneration, ImageEdit], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["gemini"], connection_role: None, endpoints: &[
        origin_endpoint(ImageGeneration, "endpoint", Submit, "POST", "/v1beta/models/{model}:generateContent"),
        origin_endpoint(ImageEdit, "endpoint", Submit, "POST", "/v1beta/models/{model}:generateContent"),
    ] },
    ProtocolSpec { id: "deepgram.listen", tasks: &[SpeechRecognition], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["deepgram"], connection_role: None, endpoints: &[origin_endpoint(SpeechRecognition, "endpoint", Submit, "POST", "/v1/listen")] },
    ProtocolSpec { id: "deepgram.speak_rest", tasks: &[SpeechSynthesis], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["deepgram"], connection_role: None, endpoints: &[origin_endpoint(SpeechSynthesis, "endpoint", Submit, "POST", "/v1/speak")] },
    ProtocolSpec { id: "ark.images", tasks: &[ImageGeneration], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["ark", "volcengine"], connection_role: None, endpoints: &[endpoint(ImageGeneration, "endpoint", Submit, "POST", "/images/generations")] },
    ProtocolSpec { id: "ark.video_jobs", tasks: &[VideoGeneration], executor: AsyncJob, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["ark", "volcengine"], connection_role: None, endpoints: &[
        endpoint(VideoGeneration, "endpoint", Submit, "POST", "/contents/generations/tasks"),
        endpoint(VideoGeneration, "poll_endpoint", Poll, "GET", "/contents/generations/tasks/{id}"),
    ] },
    ProtocolSpec { id: "volc.asr_file", tasks: &[SpeechRecognition], executor: AsyncJob, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["ark", "volcengine"], connection_role: Some("voice"), endpoints: &[
        origin_endpoint(SpeechRecognition, "endpoint", Submit, "POST", "/api/v3/auc/bigmodel/submit"),
        origin_endpoint(SpeechRecognition, "poll_endpoint", Poll, "POST", "/api/v3/auc/bigmodel/query"),
    ] },
    ProtocolSpec { id: "volc.tts_v3", tasks: &[SpeechSynthesis], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["ark", "volcengine"], connection_role: Some("voice"), endpoints: &[origin_endpoint(SpeechSynthesis, "endpoint", Submit, "POST", "/api/v3/tts/unidirectional")] },
    ProtocolSpec { id: "dashscope.images", tasks: &[ImageGeneration], executor: AsyncJob, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["dashscope"], connection_role: None, endpoints: &[
        origin_endpoint(ImageGeneration, "endpoint", Submit, "POST", "/api/v1/services/aigc/text2image/image-synthesis"),
        origin_endpoint(ImageGeneration, "poll_endpoint", Poll, "GET", "/api/v1/tasks/{id}"),
    ] },
    ProtocolSpec { id: "dashscope.embeddings", tasks: &[Embedding], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["dashscope"], connection_role: None, endpoints: &[origin_endpoint(Embedding, "endpoint", Submit, "POST", "/api/v1/services/embeddings/text-embedding/text-embedding")] },
    ProtocolSpec { id: "minimax.t2a", tasks: &[SpeechSynthesis], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["minimax"], connection_role: None, endpoints: &[endpoint(SpeechSynthesis, "endpoint", Submit, "POST", "/t2a_v2")] },
    ProtocolSpec { id: "mimo.chat_asr", tasks: &[SpeechRecognition], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["mimo"], connection_role: None, endpoints: &[endpoint(SpeechRecognition, "endpoint", Submit, "POST", "/chat/completions")] },
    ProtocolSpec { id: "mimo.chat_tts", tasks: &[SpeechSynthesis], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["mimo"], connection_role: None, endpoints: &[endpoint(SpeechSynthesis, "endpoint", Submit, "POST", "/chat/completions")] },
    ProtocolSpec { id: "siliconflow.audio_speech", tasks: &[SpeechSynthesis], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["siliconflow"], connection_role: None, endpoints: &[endpoint(SpeechSynthesis, "endpoint", Submit, "POST", "/audio/speech")] },
    ProtocolSpec { id: "siliconflow.images", tasks: &[ImageGeneration, ImageEdit], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["siliconflow"], connection_role: None, endpoints: &[
        endpoint(ImageGeneration, "endpoint", Submit, "POST", "/images/generations"),
        endpoint(ImageEdit, "endpoint", Submit, "POST", "/images/generations"),
    ] },
    ProtocolSpec { id: "siliconflow.video_jobs", tasks: &[VideoGeneration], executor: AsyncJob, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["siliconflow"], connection_role: None, endpoints: &[
        endpoint(VideoGeneration, "endpoint", Submit, "POST", "/video/submit"),
        endpoint(VideoGeneration, "poll_endpoint", Poll, "POST", "/video/status"),
    ] },
    ProtocolSpec { id: "stepfun.audio_speech", tasks: &[SpeechSynthesis], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["stepfun", "stepfun-plan"], connection_role: None, endpoints: &[endpoint(SpeechSynthesis, "endpoint", Submit, "POST", "/audio/speech")] },
    ProtocolSpec { id: "stepfun.asr_sse", tasks: &[SpeechRecognition], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["stepfun", "stepfun-plan"], connection_role: None, endpoints: &[endpoint(SpeechRecognition, "endpoint", Submit, "POST", "/audio/asr/sse")] },
    ProtocolSpec { id: "stepfun.images", tasks: &[ImageGeneration, ImageEdit], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["stepfun", "stepfun-plan"], connection_role: None, endpoints: &[
        endpoint(ImageGeneration, "endpoint", Submit, "POST", "/images/generations"),
        endpoint(ImageEdit, "endpoint", Submit, "POST", "/images/edits"),
    ] },
    ProtocolSpec { id: "stepfun.realtime_s2s", tasks: &[RealtimeConversation], executor: RealtimeSession, transport: Websocket, scopes: NATIVE_CUSTOM, platforms: &["stepfun", "stepfun-plan"], connection_role: None, endpoints: &[endpoint(RealtimeConversation, "realtime_endpoint", Session, "GET", "/realtime?model={model}")] },
    ProtocolSpec { id: "xai.images_json", tasks: &[ImageGeneration, ImageEdit], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["xai"], connection_role: None, endpoints: &[
        endpoint(ImageGeneration, "endpoint", Submit, "POST", "/images/generations"),
        endpoint(ImageEdit, "endpoint", Submit, "POST", "/images/edits"),
    ] },
    ProtocolSpec { id: "xai.video_jobs", tasks: &[VideoGeneration], executor: AsyncJob, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["xai"], connection_role: None, endpoints: &[
        endpoint(VideoGeneration, "endpoint", Submit, "POST", "/videos/generations"),
        endpoint(VideoGeneration, "poll_endpoint", Poll, "GET", "/videos/{request_id}"),
    ] },
    ProtocolSpec { id: "xai.tts", tasks: &[SpeechSynthesis], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["xai"], connection_role: None, endpoints: &[endpoint(SpeechSynthesis, "endpoint", Submit, "POST", "/tts")] },
    ProtocolSpec { id: "xai.stt", tasks: &[SpeechRecognition], executor: ModelInvoke, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["xai"], connection_role: None, endpoints: &[endpoint(SpeechRecognition, "endpoint", Submit, "POST", "/stt")] },
    ProtocolSpec { id: "zhipu.video_jobs", tasks: &[VideoGeneration], executor: AsyncJob, transport: Http, scopes: NATIVE_CUSTOM, platforms: &["zhipu"], connection_role: None, endpoints: &[
        endpoint(VideoGeneration, "endpoint", Submit, "POST", "/videos/generations"),
        endpoint(VideoGeneration, "poll_endpoint", Poll, "GET", "/async-result/{id}"),
    ] },
];

fn protocol_connection_override(
    protocol_id: &str,
    platform: &str,
) -> Option<(&'static str, &'static str)> {
    match (protocol_id, platform) {
        ("dashscope.images" | "dashscope.embeddings", "dashscope") => {
            Some(("https://dashscope.aliyuncs.com", "bearer"))
        }
        ("volc.asr_file" | "volc.tts_v3", "ark" | "volcengine") => {
            Some(("https://openspeech.bytedance.com", "volc_voice"))
        }
        _ => None,
    }
}

fn valid_placeholder_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn collect_endpoint_placeholders(value: &str) -> Result<BTreeSet<String>, InvokeError> {
    let mut placeholders = BTreeSet::new();
    let bytes = value.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        match bytes[cursor] {
            b'{' => {
                let start = cursor + 1;
                let Some(relative_end) = bytes[start..].iter().position(|byte| *byte == b'}') else {
                    return Err(InvokeError::config(
                        "endpoint template contains an unclosed placeholder",
                    ));
                };
                let end = start + relative_end;
                if bytes[start..end].contains(&b'{') {
                    return Err(InvokeError::config(
                        "endpoint template contains nested placeholders",
                    ));
                }
                let name = std::str::from_utf8(&bytes[start..end]).map_err(|_| {
                    InvokeError::config("endpoint template placeholder must be ASCII")
                })?;
                if !valid_placeholder_name(name) {
                    return Err(InvokeError::config(format!(
                        "endpoint template contains invalid placeholder {{{name}}}"
                    )));
                }
                placeholders.insert(name.to_owned());
                cursor = end + 1;
            }
            b'}' => {
                return Err(InvokeError::config(
                    "endpoint template contains an unmatched closing brace",
                ));
            }
            _ => cursor += 1,
        }
    }
    Ok(placeholders)
}

/// Validate one configured endpoint override against the exact protocol field
/// descriptor. This is the single save/runtime authority for placeholder
/// syntax and vocabulary.
pub fn validate_endpoint_template(
    protocol_id: &str,
    task: ModelTask,
    field: &str,
    value: &str,
) -> Result<(), InvokeError> {
    if value.trim().is_empty() {
        return Err(InvokeError::config(format!(
            "protocol {protocol_id:?} endpoint field {field:?} cannot be blank"
        )));
    }
    let descriptor = protocol_task_descriptor(protocol_id, task).ok_or_else(|| {
        InvokeError::config(format!(
            "unknown or task-incompatible protocol {protocol_id:?} for {task:?}"
        ))
    })?;
    let endpoint = descriptor
        .endpoints
        .iter()
        .find(|endpoint| endpoint.field == field)
        .ok_or_else(|| {
            InvokeError::config(format!(
                "protocol {protocol_id:?} does not define endpoint field {field:?} for {task:?}"
            ))
        })?;
    let actual = collect_endpoint_placeholders(value)?;
    let allowed = endpoint
        .allowed_placeholders
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if let Some(unknown) = actual.difference(&allowed).next() {
        return Err(InvokeError::config(format!(
            "protocol {protocol_id:?} endpoint field {field:?} does not allow placeholder {{{unknown}}}"
        )));
    }
    let required = endpoint
        .required_placeholders
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if !required.is_empty() && actual.is_disjoint(&required) {
        return Err(InvokeError::config(format!(
            "protocol {protocol_id:?} endpoint field {field:?} must contain one of: {}",
            required
                .iter()
                .map(|name| format!("{{{name}}}"))
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    validate_openai_chat_endpoint_owner(protocol_id, value)?;
    Ok(())
}

fn validate_openai_chat_endpoint_owner(
    protocol_id: &str,
    value: &str,
) -> Result<(), InvokeError> {
    if !matches!(protocol_id, "openai.chat_text" | "openai.responses") {
        return Ok(());
    }
    let without_suffix = value
        .trim()
        .split(['?', '#'])
        .next()
        .unwrap_or_default();
    let path = reqwest::Url::parse(without_suffix)
        .ok()
        .map(|url| url.path().to_owned())
        .unwrap_or_else(|| without_suffix.to_owned());
    let path = path.trim_end_matches('/').to_ascii_lowercase();
    let is_responses = path == "responses" || path.ends_with("/responses");
    let is_chat_completions =
        path == "chat/completions" || path.ends_with("/chat/completions");
    if protocol_id == "openai.chat_text" && is_responses {
        return Err(InvokeError::config(
            "openai.chat_text cannot target a /responses endpoint; select openai.responses",
        ));
    }
    if protocol_id == "openai.responses" && is_chat_completions {
        return Err(InvokeError::config(
            "openai.responses cannot target a /chat/completions endpoint; select openai.chat_text",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderParamsEncoding {
    /// Provider-native JSON fields are recursively merged into the request;
    /// typed task fields are applied last.
    Json,
    /// Fields are encoded losslessly as multipart/query text. Only JSON
    /// strings, numbers and booleans have an unambiguous representation.
    ScalarFields,
}

fn provider_params_encoding(
    protocol_id: &str,
    task: ModelTask,
) -> Option<ProviderParamsEncoding> {
    use ProviderParamsEncoding::{Json, ScalarFields};

    Some(match (protocol_id, task) {
        ("openai.images", ImageEdit)
        | ("openai.videos", VideoGeneration)
        | ("openai.audio_transcriptions", SpeechRecognition)
        | ("deepgram.listen", SpeechRecognition)
        | ("deepgram.speak_rest", SpeechSynthesis)
        | ("stepfun.images", ImageEdit)
        | ("xai.stt", SpeechRecognition) => ScalarFields,

        ("openai.chat_text", Chat)
        | ("openai.responses", Chat)
        | ("anthropic.messages", Chat)
        | ("bedrock.anthropic_messages", Chat)
        | ("gemini.generate_text", Chat)
        | ("openai.images", ImageGeneration)
        | ("openai.embeddings", Embedding)
        | ("generic.rerank", Rerank)
        | ("openai.audio_speech", SpeechSynthesis)
        | ("gemini.generate_content", ImageGeneration | ImageEdit)
        | ("ark.images", ImageGeneration)
        | ("ark.video_jobs", VideoGeneration)
        | ("volc.asr_file", SpeechRecognition)
        | ("volc.tts_v3", SpeechSynthesis)
        | ("dashscope.images", ImageGeneration)
        | ("dashscope.embeddings", Embedding)
        | ("minimax.t2a", SpeechSynthesis)
        | ("mimo.chat_asr", SpeechRecognition)
        | ("mimo.chat_tts", SpeechSynthesis)
        | ("siliconflow.audio_speech", SpeechSynthesis)
        | ("siliconflow.images", ImageGeneration | ImageEdit)
        | ("siliconflow.video_jobs", VideoGeneration)
        | ("stepfun.audio_speech", SpeechSynthesis)
        | ("stepfun.asr_sse", SpeechRecognition)
        | ("stepfun.images", ImageGeneration)
        | ("stepfun.realtime_s2s", RealtimeConversation)
        | ("xai.images_json", ImageGeneration | ImageEdit)
        | ("xai.video_jobs", VideoGeneration)
        | ("xai.tts", SpeechSynthesis)
        | ("zhipu.video_jobs", VideoGeneration) => Json,

        _ => return None,
    })
}

/// Validate the lossless provider-parameter encoding contract for one exact
/// protocol/task pair. Management APIs call this before persistence and the
/// resolver repeats it for corrupt or pre-contract rows, so a parameter can
/// never save successfully and then be silently discarded by an executor.
pub fn validate_provider_params_for_protocol(
    protocol_id: &str,
    task: ModelTask,
    params: &serde_json::Value,
) -> Result<(), InvokeError> {
    if protocol_task_descriptor(protocol_id, task).is_none() {
        return Err(InvokeError::config(format!(
            "unknown or task-incompatible protocol {protocol_id:?} for {task:?}"
        )));
    }
    let object = params.as_object().ok_or_else(|| {
        InvokeError::config("capability provider_params must be a JSON object")
    })?;
    if let Some(key) = object
        .keys()
        .find(|key| is_reserved_local_transport_param_key(key))
    {
        return Err(InvokeError::config(format!(
            "capability provider_params contains reserved local transport/auth field {key:?}"
        )));
    }
    // This historical StepFun adapter-control hint is deliberately consumed
    // nowhere. Reject it at save time instead of accepting a no-op field or
    // leaking it upstream.
    if protocol_id == "stepfun.images" && object.contains_key("generation_option_keys") {
        return Err(InvokeError::config(
            "stepfun.images provider_params field \"generation_option_keys\" is not a provider request field",
        ));
    }
    if let Some(chain_rounds) = object.get("chain_rounds") {
        if protocol_id != "openai.responses" || task != Chat {
            return Err(InvokeError::config(
                "provider_params.chain_rounds is supported only by openai.responses Chat capabilities",
            ));
        }
        if !chain_rounds.is_boolean() {
            return Err(InvokeError::config(
                "openai.responses provider_params.chain_rounds must be a boolean",
            ));
        }
    }
    if task == Chat {
        let configured_ceiling_key = match object.get("max_tokens_field") {
            None => None,
            Some(serde_json::Value::String(value)) if !value.trim().is_empty() => {
                Some(value.trim())
            }
            Some(_) => {
                return Err(InvokeError::config(
                    "Chat provider_params.max_tokens_field must be a non-empty string",
                ));
            }
        };
        const OUTPUT_CEILING_KEYS: &[&str] = &[
            "max_tokens",
            "max_completion_tokens",
            "maxOutputTokens",
            "max_output_tokens",
        ];
        let shadow_key = OUTPUT_CEILING_KEYS
            .iter()
            .copied()
            .find(|key| object.contains_key(*key))
            .or_else(|| configured_ceiling_key.filter(|key| object.contains_key(*key)));
        if let Some(key) = shadow_key {
            return Err(InvokeError::config(format!(
                "Chat provider_params must not set {key:?}; use the capability's output_limit field"
            )));
        }
        if object
            .get("generationConfig")
            .and_then(serde_json::Value::as_object)
            .is_some_and(|config| config.contains_key("maxOutputTokens"))
        {
            return Err(InvokeError::config(
                "Chat provider_params must not set generationConfig.maxOutputTokens; use the capability's output_limit field",
            ));
        }
        if object.contains_key("require_reasoning_content")
            && !object
                .get("require_reasoning_content")
                .is_some_and(serde_json::Value::is_boolean)
        {
            return Err(InvokeError::config(
                "Chat provider_params.require_reasoning_content must be a boolean",
            ));
        }
    }

    match provider_params_encoding(protocol_id, task).ok_or_else(|| {
        InvokeError::config(format!(
            "protocol {protocol_id:?} has no provider_params encoding contract for {task:?}"
        ))
    })? {
        ProviderParamsEncoding::Json => Ok(()),
        ProviderParamsEncoding::ScalarFields => {
            if let Some((key, value)) = object.iter().find(|(key, value)| {
                if protocol_id == "xai.stt" && key.as_str() == "keyterm" {
                    return match value {
                        serde_json::Value::String(_) => false,
                        serde_json::Value::Array(values) => {
                            values.is_empty()
                                || values.iter().any(|entry| {
                                    entry
                                        .as_str()
                                        .is_none_or(|entry| entry.trim().is_empty())
                                })
                        }
                        _ => true,
                    };
                }
                !matches!(value, serde_json::Value::String(_) | serde_json::Value::Number(_) | serde_json::Value::Bool(_))
            }) {
                return Err(InvokeError::config(format!(
                    "protocol {protocol_id:?} encodes provider_params as multipart/query scalar fields; field {key:?} cannot losslessly encode JSON {}",
                    match value {
                        serde_json::Value::Null => "null",
                        serde_json::Value::Array(_) => "array",
                        serde_json::Value::Object(_) => "object",
                        _ => unreachable!("scalar values were excluded"),
                    }
                )));
            }
            Ok(())
        }
    }
}

/// Validate and expand a protocol-owned endpoint template. Every placeholder
/// present in the selected descriptor field identifies the same runtime value
/// (a model id on submit/session fields or a remote job id on poll/content
/// fields), and is encoded as an opaque URL component before substitution.
pub fn expand_protocol_endpoint_template(
    protocol_id: &str,
    task: ModelTask,
    field: &str,
    value: &str,
    replacement: &str,
) -> Result<String, InvokeError> {
    validate_endpoint_template(protocol_id, task, field, value)?;
    let placeholders = collect_endpoint_placeholders(value)?;
    let encoded = url::form_urlencoded::byte_serialize(replacement.as_bytes()).collect::<String>();
    let mut expanded = value.to_owned();
    for name in placeholders {
        expanded = expanded.replace(&format!("{{{name}}}"), &encoded);
    }
    Ok(expanded)
}

fn owned_endpoint(protocol_id: &str, spec: EndpointSpec) -> ProtocolEndpointDescriptor {
    let mut allowed_placeholders = collect_endpoint_placeholders(spec.default_value)
        .expect("built-in endpoint templates are valid")
        .into_iter()
        .collect::<Vec<_>>();
    // Zhipu has used both names for the same remote async identifier. The
    // manifest explicitly owns that vocabulary; save-time and runtime code do
    // not carry a second provider-specific rule.
    if protocol_id == "zhipu.video_jobs" && spec.field == "poll_endpoint" {
        allowed_placeholders.push("task_id".to_owned());
    }
    allowed_placeholders.sort();
    allowed_placeholders.dedup();
    let required_placeholders = if matches!(
        spec.purpose,
        ProtocolEndpointPurpose::Poll | ProtocolEndpointPurpose::Content
    ) {
        allowed_placeholders.clone()
    } else {
        Vec::new()
    };
    ProtocolEndpointDescriptor {
        task: spec.task,
        field: spec.field.to_owned(),
        purpose: spec.purpose,
        method: spec.method.map(str::to_owned),
        default_value: spec.default_value.to_owned(),
        root_shape: spec.root,
        allowed_placeholders,
        required_placeholders,
        editable: spec.editable,
    }
}

fn owned_protocol(spec: ProtocolSpec) -> ProtocolDescriptor {
    let mut default_connections = Vec::new();
    for preset in PRESETS {
        if !spec.platforms.contains(&preset.platform) {
            continue;
        }
        let override_connection = protocol_connection_override(spec.id, preset.platform);
        let base_url = override_connection.map(|value| value.0).or(preset.base_url);
        let auth_scheme = override_connection.map(|value| value.1).or(preset.auth_scheme);
        if let (Some(base_url), Some(auth_scheme)) = (base_url, auth_scheme) {
            default_connections.push(ProtocolDefaultConnection {
                preset: preset.preset.to_owned(),
                platform: preset.platform.to_owned(),
                connection_role: spec.connection_role.map(str::to_owned),
                connection_label: spec
                    .connection_role
                    .map(|role| if role == "voice" { "Volcengine Voice" } else { role })
                    .map(str::to_owned),
                base_url: base_url.to_owned(),
                auth_scheme: auth_scheme.to_owned(),
                requires_credentials: true,
            });
        }
    }
    let endpoints = spec
        .endpoints
        .iter()
        .copied()
        .map(|endpoint| owned_endpoint(spec.id, endpoint))
        .collect::<Vec<_>>();
    // Every endpoint of one protocol shares a convention (enforced by
    // `ProtocolManifestRegistry::try_new`), so the protocol-level shape is just
    // the first endpoint's. `sdk` transports build no URL and declare none.
    let root_shape = endpoints.first().map(|endpoint| endpoint.root_shape);
    ProtocolDescriptor {
        protocol_id: spec.id.to_owned(),
        supported_tasks: spec.tasks.to_vec(),
        executor: spec.executor,
        transport: spec.transport,
        requires_output_ceiling: protocol_requires_output_ceiling(spec.id),
        allowed_auth_schemes: allowed_auth_schemes(spec)
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        scopes: spec.scopes.to_vec(),
        platforms: spec.platforms.iter().map(|value| (*value).to_owned()).collect(),
        default_connections,
        endpoints,
        root_shape,
    }
}

pub fn try_default_protocol_registry() -> Result<ProtocolManifestRegistry, InvokeError> {
    let registry = ProtocolManifestRegistry::try_new(
        PROTOCOL_SPECS.iter().copied().map(owned_protocol).collect(),
    )?;

    let request_registry = AdapterRegistry::try_new(default_adapters())?;
    for protocol_id in request_registry.protocol_ids() {
        let descriptor = registry.get(protocol_id).ok_or_else(|| {
            InvokeError::config(format!(
                "registered request adapter {protocol_id:?} has no protocol descriptor"
            ))
        })?;
        for task in ALL_MODEL_TASKS {
            let advertised = descriptor.supported_tasks.contains(&task);
            let implemented = request_registry.get(protocol_id, task).is_ok();
            if advertised != implemented {
                return Err(InvokeError::config(format!(
                    "protocol descriptor task mismatch for {protocol_id:?} and {task:?}"
                )));
            }
        }
    }

    let realtime_registry = RealtimeAdapterRegistry::try_new(default_realtime_adapters())?;
    for protocol_id in realtime_registry.protocol_ids() {
        if request_registry.contains(protocol_id) {
            return Err(InvokeError::config(format!(
                "protocol id {protocol_id:?} is registered in both request and realtime registries"
            )));
        }
        let descriptor = registry.get(protocol_id).ok_or_else(|| {
            InvokeError::config(format!(
                "registered realtime adapter {protocol_id:?} has no protocol descriptor"
            ))
        })?;
        if descriptor.executor != ProtocolExecutorKind::RealtimeSession
            || descriptor.supported_tasks != [ModelTask::RealtimeConversation]
        {
            return Err(InvokeError::config(format!(
                "realtime descriptor {protocol_id:?} must serve only realtime_conversation"
            )));
        }
    }

    for descriptor in registry.descriptors() {
        let valid_executor = match descriptor.executor {
            ProtocolExecutorKind::Agent => matches!(
                descriptor.protocol_id.as_str(),
                "openai.chat_text"
                    | "openai.responses"
                    | "anthropic.messages"
                    | "bedrock.anthropic_messages"
                    | "gemini.generate_text"
            ),
            ProtocolExecutorKind::ModelInvoke | ProtocolExecutorKind::AsyncJob => {
                request_registry.contains(&descriptor.protocol_id)
            }
            ProtocolExecutorKind::RealtimeSession => realtime_registry.contains(&descriptor.protocol_id),
        };
        if !valid_executor {
            return Err(InvokeError::config(format!(
                "protocol descriptor {:?} has no matching executor registration",
                descriptor.protocol_id
            )));
        }
    }
    Ok(registry)
}

pub fn default_protocol_registry() -> ProtocolManifestRegistry {
    try_default_protocol_registry()
        .unwrap_or_else(|error| panic!("invalid default protocol manifest: {error}"))
}

pub fn protocol_descriptor(protocol_id: &str) -> Option<ProtocolDescriptor> {
    default_protocol_registry().get(protocol_id).cloned()
}

pub fn protocol_task_descriptor(
    protocol_id: &str,
    task: ModelTask,
) -> Option<ProtocolTaskDescriptor> {
    let descriptor = protocol_descriptor(protocol_id)?;
    if !descriptor.supported_tasks.contains(&task) {
        return None;
    }
    Some(ProtocolTaskDescriptor {
        protocol_id: descriptor.protocol_id,
        task,
        executor: descriptor.executor,
        transport: descriptor.transport,
        endpoints: descriptor
            .endpoints
            .into_iter()
            .filter(|endpoint| endpoint.task == task)
            .collect(),
        root_shape: descriptor.root_shape,
    })
}

pub fn auth_scheme_descriptors() -> Vec<AuthSchemeDescriptor> {
    [
        ("bearer", false),
        ("token", false),
        ("header_key:<name>", true),
        ("query_key:<param>", true),
        ("volc_voice", false),
        ("bedrock", false),
    ]
    .into_iter()
    .map(|(scheme, parameterized)| AuthSchemeDescriptor { scheme: scheme.to_owned(), parameterized })
    .collect()
}

pub fn protocol_manifest_for(preset: &str, task: ModelTask) -> ModelProtocolManifestResponse {
    protocol_manifest_for_connection(preset, None, task)
}

pub fn protocol_manifest_for_connection(
    preset: &str,
    base_url_hint: Option<&str>,
    task: ModelTask,
) -> ModelProtocolManifestResponse {
    protocol_manifest_for_model_connection(preset, base_url_hint, None, task)
}

/// Build configuration-time protocol metadata with an optional model-id hint.
///
/// The model id is deliberately only a signal that the user has entered or
/// selected a concrete model. It is never parsed to infer a vendor or protocol.
/// For the `custom` preset, that signal allows the manifest to preselect the
/// sole registry-declared generic compatibility protocol for the requested
/// task. Callers still have to persist the selected protocol explicitly; this
/// function is not consulted by runtime resolution or probing.
pub fn protocol_manifest_for_model_connection(
    preset: &str,
    base_url_hint: Option<&str>,
    model_hint: Option<&str>,
    task: ModelTask,
) -> ModelProtocolManifestResponse {
    let selected = resolve_preset(preset, base_url_hint);
    let custom_scope = matches!(selected.platform.as_str(), "custom" | "new-api");
    let registry = default_protocol_registry();
    let mut protocols = registry
        .descriptors()
        .filter(|descriptor| descriptor.supported_tasks.contains(&task))
        .filter(|descriptor| {
            descriptor.platforms.iter().any(|platform| platform == &selected.platform)
                || descriptor.scopes.contains(&ProtocolScope::Custom)
        })
        .cloned()
        .map(|mut descriptor| {
            let platform_match = descriptor
                .platforms
                .iter()
                .any(|platform| platform == &selected.platform);
            if !custom_scope && platform_match {
                descriptor
                    .default_connections
                    .retain(|connection| connection.preset == selected.preset);
            } else {
                // Cross-provider Custom protocols are explicit escape hatches:
                // advertise the serializer, but never guess credentials or a
                // provider endpoint on the user's behalf.
                descriptor.default_connections.clear();
            }
            descriptor
        })
        .collect::<Vec<_>>();
    protocols.sort_by(|left, right| {
        let left_matches = left.platforms.iter().any(|platform| platform == &selected.platform);
        let right_matches = right.platforms.iter().any(|platform| platform == &selected.platform);
        right_matches
            .cmp(&left_matches)
            .then_with(|| left.protocol_id.cmp(&right.protocol_id))
    });

    let recommendation = if selected.platform == "custom"
        && model_hint.is_some_and(|model| !model.trim().is_empty())
    {
        generic_custom_protocol_recommendation(&registry, &selected, task)
    } else if custom_scope {
        None
    } else {
        preset_protocol_recommendation(&selected.platform, task).and_then(|route| {
            let descriptor = registry.get(route.protocol)?;
            if !descriptor.supported_tasks.contains(&task)
                || !descriptor.platforms.iter().any(|platform| platform == &selected.platform)
            {
                return None;
            }
            let connection = descriptor
                .default_connections
                .iter()
                .find(|connection| connection.preset == selected.preset)
                .or_else(|| {
                    descriptor
                        .default_connections
                        .iter()
                        .find(|connection| connection.platform == selected.platform)
                });
            Some(ProtocolRecommendation {
                protocol_id: route.protocol.to_owned(),
                connection_role: route.connection_role.map(str::to_owned),
                default_base_url: connection
                    .map(|value| value.base_url.clone())
                    .or_else(|| selected.platform_default_base_url.clone()),
                default_auth_scheme: connection
                    .map(|value| value.auth_scheme.clone())
                    .or_else(|| selected.default_auth_scheme.clone()),
                base_url_override_required: route.connection_role.is_none()
                    && connection.is_some_and(|connection| {
                        selected.platform_default_base_url.as_deref()
                            != Some(connection.base_url.as_str())
                    }),
            })
        })
    };

    ModelProtocolManifestResponse {
        tasks: ALL_MODEL_TASKS.to_vec(),
        preset: selected.preset,
        platform: selected.platform,
        requested_task: task,
        platform_default_base_url: selected.platform_default_base_url,
        requires_user_input: selected.requires_user_input,
        default_auth_scheme: selected.default_auth_scheme,
        auth_schemes: auth_scheme_descriptors(),
        recommendation,
        protocols,
    }
}

/// Recommend only an unambiguous, registry-declared generic compatibility
/// protocol. Requiring both scopes keeps provider-native escape hatches out of
/// the default path, and requiring exactly one match makes registry expansion
/// fail closed instead of silently changing a user's new-model configuration.
fn generic_custom_protocol_recommendation(
    registry: &ProtocolManifestRegistry,
    selected: &PlatformPresetDescriptor,
    task: ModelTask,
) -> Option<ProtocolRecommendation> {
    let mut candidates = registry.descriptors().filter(|descriptor| {
        descriptor.supported_tasks.contains(&task)
            && descriptor.scopes.contains(&ProtocolScope::OfficialCompat)
            && descriptor.scopes.contains(&ProtocolScope::Custom)
    });
    let descriptor = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }

    let default_auth_scheme = selected.default_auth_scheme.as_ref().and_then(|scheme| {
        descriptor
            .allowed_auth_schemes
            .iter()
            .any(|allowed| allowed == scheme)
            .then(|| scheme.clone())
    });
    Some(ProtocolRecommendation {
        protocol_id: descriptor.protocol_id.clone(),
        connection_role: None,
        default_base_url: None,
        default_auth_scheme,
        base_url_override_required: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_descriptor(id: &str) -> ProtocolDescriptor {
        ProtocolDescriptor {
            protocol_id: id.to_owned(),
            supported_tasks: vec![Chat],
            executor: Agent,
            transport: Http,
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
    fn manifest_registry_rejects_duplicate_protocol_ids() {
        let error = ProtocolManifestRegistry::try_new(vec![
            fake_descriptor("duplicate"),
            fake_descriptor("duplicate"),
        ])
        .unwrap_err();
        assert!(error.message.contains("duplicate protocol descriptor"));
    }

    #[test]
    fn generic_custom_recommendation_requires_exactly_one_dual_scope_candidate() {
        let selected = resolve_preset("custom", None);
        let mut generic = fake_descriptor("generic.chat");
        generic.scopes = vec![ProtocolScope::OfficialCompat, ProtocolScope::Custom];
        let registry = ProtocolManifestRegistry::try_new(vec![generic]).unwrap();
        let recommendation = generic_custom_protocol_recommendation(&registry, &selected, Chat)
            .expect("one generic compatibility protocol");
        assert_eq!(recommendation.protocol_id, "generic.chat");
        assert_eq!(recommendation.default_auth_scheme.as_deref(), Some("bearer"));

        let mut first = fake_descriptor("generic.chat.first");
        first.scopes = vec![ProtocolScope::OfficialCompat, ProtocolScope::Custom];
        let mut second = fake_descriptor("generic.chat.second");
        second.scopes = vec![ProtocolScope::OfficialCompat, ProtocolScope::Custom];
        let registry = ProtocolManifestRegistry::try_new(vec![
            fake_descriptor("native.escape-hatch"),
            first,
            second,
        ])
        .unwrap();
        assert!(generic_custom_protocol_recommendation(&registry, &selected, Chat).is_none());
    }

    #[test]
    fn manifest_registry_rejects_duplicate_endpoint_keys() {
        let mut descriptor = fake_descriptor("duplicate.endpoint");
        let repeated = ProtocolEndpointDescriptor {
            task: Chat,
            field: "endpoint".to_owned(),
            purpose: Submit,
            method: Some("POST".to_owned()),
            default_value: "/chat/completions".to_owned(),
            root_shape: EndpointRootShape::VersionedRoot,
            allowed_placeholders: vec![],
            required_placeholders: vec![],
            editable: true,
        };
        descriptor.endpoints = vec![repeated.clone(), repeated];
        let error = ProtocolManifestRegistry::try_new(vec![descriptor]).unwrap_err();
        assert!(error.message.contains("duplicate endpoint"));
    }

    #[test]
    fn default_manifest_matches_both_executor_registries() {
        let registry = try_default_protocol_registry().expect("default manifest must be consistent");
        assert!(registry.get("stepfun.images").is_some());
        assert!(registry.get("stepfun.realtime_s2s").is_some());
        assert!(registry.get("anthropic.messages").is_some());
        assert!(registry.get("bedrock.anthropic_messages").is_some());
        assert!(registry.get("gemini.generate_text").is_some());
        assert!(registry.get("openai.responses").is_some());
    }

    #[test]
    fn every_manifest_protocol_task_has_a_provider_params_encoding_contract() {
        let registry = default_protocol_registry();
        for descriptor in registry.descriptors() {
            for task in &descriptor.supported_tasks {
                validate_provider_params_for_protocol(
                    &descriptor.protocol_id,
                    *task,
                    &serde_json::json!({}),
                )
                .unwrap_or_else(|error| {
                    panic!(
                        "missing provider_params contract for {}/{task:?}: {error}",
                        descriptor.protocol_id
                    )
                });
            }
        }
    }

    #[test]
    fn provider_params_encoding_rejects_values_the_executor_cannot_send() {
        validate_provider_params_for_protocol(
            "openai.chat_text",
            Chat,
            &serde_json::json!({"future":{"nested":true}}),
        )
        .unwrap();
        validate_provider_params_for_protocol(
            "openai.audio_transcriptions",
            SpeechRecognition,
            &serde_json::json!({"temperature":0.2,"prompt":"hello","diarize":true}),
        )
        .unwrap();
        validate_provider_params_for_protocol(
            "xai.stt",
            SpeechRecognition,
            &serde_json::json!({"keyterm":["NomiFun","StepFun"],"temperature":0.2}),
        )
        .unwrap();

        for unsupported in [
            serde_json::Value::Null,
            serde_json::json!(["a", "b"]),
            serde_json::json!({"nested":true}),
        ] {
            let error = validate_provider_params_for_protocol(
                "openai.audio_transcriptions",
                SpeechRecognition,
                &serde_json::json!({"future":unsupported}),
            )
            .unwrap_err();
            assert!(error.message.contains("cannot losslessly encode"));
        }

        for malformed_keyterm in [
            serde_json::json!([]),
            serde_json::json!(["ok", ""]),
            serde_json::json!(["ok", 2]),
        ] {
            assert!(
                validate_provider_params_for_protocol(
                    "xai.stt",
                    SpeechRecognition,
                    &serde_json::json!({"keyterm":malformed_keyterm}),
                )
                .is_err()
            );
        }

        validate_provider_params_for_protocol(
            "bedrock.anthropic_messages",
            Chat,
            &serde_json::json!({"temperature":0.2,"stop_sequences":["END"]}),
        )
        .unwrap();
    }

    #[test]
    fn provider_params_encoding_rejects_local_and_no_op_control_fields() {
        let local = validate_provider_params_for_protocol(
            "openai.chat_text",
            Chat,
            &serde_json::json!({"api_key":"must-not-leak"}),
        )
        .unwrap_err();
        assert!(local.message.contains("reserved local transport/auth"));

        let no_op = validate_provider_params_for_protocol(
            "stepfun.images",
            ImageGeneration,
            &serde_json::json!({"generation_option_keys":["negative_prompt"]}),
        )
        .unwrap_err();
        assert!(no_op.message.contains("not a provider request field"));

        for malformed in [
            serde_json::json!({"max_tokens_field":" "}),
            serde_json::json!({"max_tokens_field":1}),
            serde_json::json!({"require_reasoning_content":"true"}),
        ] {
            assert!(
                validate_provider_params_for_protocol("openai.chat_text", Chat, &malformed)
                    .is_err()
            );
        }

        for ceiling in [
            serde_json::json!({"max_tokens":8192}),
            serde_json::json!({"max_completion_tokens":8192}),
            serde_json::json!({"maxOutputTokens":8192}),
            serde_json::json!({"max_output_tokens":8192}),
            serde_json::json!({"max_tokens_field":"custom_limit","custom_limit":8192}),
            serde_json::json!({"generationConfig":{"maxOutputTokens":8192}}),
        ] {
            let error =
                validate_provider_params_for_protocol("openai.chat_text", Chat, &ceiling)
                    .unwrap_err();
            assert!(error.message.contains("output_limit"), "{error:?}");
        }

        validate_provider_params_for_protocol(
            "siliconflow.audio_speech",
            SpeechSynthesis,
            &serde_json::json!({"max_tokens":128}),
        )
        .unwrap();
    }

    #[test]
    fn response_chaining_is_a_typed_responses_only_chat_control() {
        for value in [serde_json::json!(true), serde_json::json!(false)] {
            validate_provider_params_for_protocol(
                "openai.responses",
                Chat,
                &serde_json::json!({"chain_rounds": value}),
            )
            .unwrap();
        }
        for (protocol, task, value) in [
            ("openai.responses", Chat, serde_json::json!("true")),
            ("openai.chat_text", Chat, serde_json::json!(true)),
            ("openai.images", ImageGeneration, serde_json::json!(true)),
        ] {
            let error = validate_provider_params_for_protocol(
                protocol,
                task,
                &serde_json::json!({"chain_rounds": value}),
            )
            .unwrap_err();
            assert!(error.message.contains("chain_rounds"), "{error:?}");
        }
    }

    #[test]
    fn output_ceiling_requirement_matches_registered_chat_protocols() {
        assert!(protocol_requires_output_ceiling("anthropic.messages"));
        assert!(protocol_requires_output_ceiling(
            "bedrock.anthropic_messages"
        ));
        assert!(!protocol_requires_output_ceiling("openai.chat_text"));
        assert!(!protocol_requires_output_ceiling("openai.responses"));
        assert!(!protocol_requires_output_ceiling("gemini.generate_text"));
    }

    #[test]
    fn endpoint_template_validation_is_the_protocol_placeholder_authority() {
        // Model-scoped submit URLs may be supplied as an already exact URL;
        // `{model}` is allowed but is not a dynamic job-resume requirement.
        validate_endpoint_template(
            "gemini.generate_content",
            ImageGeneration,
            "endpoint",
            "/v1beta/models/gemini-3-pro:generateContent",
        )
        .unwrap();
        validate_endpoint_template(
            "openai.videos",
            VideoGeneration,
            "poll_endpoint",
            "/videos/{id}",
        )
        .unwrap();
        assert!(
            validate_endpoint_template(
                "openai.videos",
                VideoGeneration,
                "poll_endpoint",
                "/videos/fixed",
            )
            .is_err()
        );
        assert!(
            validate_endpoint_template(
                "openai.videos",
                VideoGeneration,
                "poll_endpoint",
                "/videos/{request_id}",
            )
            .is_err()
        );
        for placeholder in ["id", "task_id"] {
            validate_endpoint_template(
                "zhipu.video_jobs",
                VideoGeneration,
                "poll_endpoint",
                &format!("/async-result/{{{placeholder}}}"),
            )
            .unwrap();
        }
        for malformed in ["/videos/{id", "/videos/id}", "/videos/{{id}}"] {
            assert!(
                validate_endpoint_template(
                    "openai.videos",
                    VideoGeneration,
                    "poll_endpoint",
                    malformed,
                )
                .is_err(),
                "accepted malformed template: {malformed}"
            );
        }
        validate_endpoint_template(
            "siliconflow.video_jobs",
            VideoGeneration,
            "poll_endpoint",
            "/video/status",
        )
        .unwrap();
        assert!(
            validate_endpoint_template(
                "siliconflow.video_jobs",
                VideoGeneration,
                "poll_endpoint",
                "/video/status/{id}",
            )
            .is_err()
        );
    }

    #[test]
    fn endpoint_template_expansion_encodes_the_dynamic_component() {
        assert_eq!(
            expand_protocol_endpoint_template(
                "zhipu.video_jobs",
                VideoGeneration,
                "poll_endpoint",
                "/async-result/{task_id}",
                "job/a?b&c",
            )
            .unwrap(),
            "/async-result/job%2Fa%3Fb%26c"
        );
    }

    #[test]
    fn openai_chat_endpoints_cannot_cross_protocol_owners() {
        for responses_path in [
            "/responses",
            "/RESPONSES/?trace=1#fragment",
            "https://api.example.test/v1/responses?trace=1",
        ] {
            let error = validate_endpoint_template(
                "openai.chat_text",
                Chat,
                "endpoint",
                responses_path,
            )
            .unwrap_err();
            assert!(error.message.contains("openai.responses"), "{error:?}");
        }
        for chat_path in [
            "/chat/completions",
            "/CHAT/COMPLETIONS/?trace=1#fragment",
            "https://api.example.test/v1/chat/completions?trace=1",
        ] {
            let error = validate_endpoint_template(
                "openai.responses",
                Chat,
                "endpoint",
                chat_path,
            )
            .unwrap_err();
            assert!(error.message.contains("openai.chat_text"), "{error:?}");
        }
        validate_endpoint_template(
            "openai.chat_text",
            Chat,
            "endpoint",
            "/vendor/custom-chat",
        )
        .unwrap();
        validate_endpoint_template(
            "openai.responses",
            Chat,
            "endpoint",
            "/vendor/custom-responses",
        )
        .unwrap();
    }

    #[test]
    fn protocol_auth_schemes_match_strict_agents_and_flexible_http_transport() {
        let registry = default_protocol_registry();
        for (protocol, expected) in [
            ("openai.chat_text", vec!["bearer"]),
            ("openai.responses", vec!["bearer"]),
            ("anthropic.messages", vec!["header_key:x-api-key"]),
            ("gemini.generate_text", vec!["header_key:x-goog-api-key"]),
            ("bedrock.anthropic_messages", vec!["bedrock"]),
            ("stepfun.realtime_s2s", vec!["bearer"]),
            ("volc.asr_file", vec!["volc_voice"]),
        ] {
            assert_eq!(
                registry.get(protocol).expect("descriptor").allowed_auth_schemes,
                expected
            );
        }
        let images = registry.get("openai.images").expect("descriptor");
        assert_eq!(
            images.allowed_auth_schemes,
            ["bearer", "token", "header_key:<name>", "query_key:<param>"]
        );
    }

    #[test]
    fn every_non_sdk_endpoint_is_user_editable() {
        let registry = default_protocol_registry();
        for descriptor in registry
            .descriptors()
            .filter(|descriptor| descriptor.transport != Sdk)
        {
            for endpoint in &descriptor.endpoints {
                assert!(
                    endpoint.editable,
                    "{} {:?} {} must remain editable for provider API evolution",
                    descriptor.protocol_id, endpoint.purpose, endpoint.field
                );
            }
        }
    }

    #[test]
    fn response_prioritizes_platform_protocols_and_exposes_custom_escape_hatches() {
        let stepfun = protocol_manifest_for("StepFun", RealtimeConversation);
        assert_eq!(stepfun.tasks, ALL_MODEL_TASKS);
        assert_eq!(stepfun.protocols.len(), 1);
        assert_eq!(stepfun.protocols[0].protocol_id, "stepfun.realtime_s2s");
        assert_eq!(stepfun.recommendation.unwrap().protocol_id, "stepfun.realtime_s2s");

        let deepgram = protocol_manifest_for("Deepgram", RealtimeConversation);
        assert_eq!(deepgram.protocols.len(), 1);
        assert_eq!(deepgram.protocols[0].protocol_id, "stepfun.realtime_s2s");
        assert!(deepgram.protocols[0].default_connections.is_empty());
        assert!(deepgram.recommendation.is_none());
    }

    #[test]
    fn unsupported_preset_tasks_keep_custom_protocols_without_guessed_defaults() {
        for (preset, task, expected_protocol) in [
            ("Deepgram", ImageGeneration, "openai.images"),
            ("StepFun", Embedding, "openai.embeddings"),
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
    fn custom_has_all_configurable_task_protocols_but_no_default() {
        let view = protocol_manifest_for("custom", Chat);
        assert!(view.recommendation.is_none());
        assert_eq!(
            view.protocols.iter().map(|value| value.protocol_id.as_str()).collect::<Vec<_>>(),
            vec![
                "anthropic.messages",
                "gemini.generate_text",
                "openai.chat_text"
            ]
        );
    }

    #[test]
    fn openai_exposes_native_responses_without_changing_the_chat_recommendation() {
        let view = protocol_manifest_for("OpenAI", Chat);
        assert!(
            view.protocols
                .iter()
                .any(|descriptor| descriptor.protocol_id == "openai.responses")
        );
        let recommendation = view.recommendation.expect("OpenAI Chat recommendation");
        assert_eq!(recommendation.protocol_id, "openai.chat_text");
    }

    #[test]
    fn gemini_chat_uses_native_agent_protocol_and_api_key_auth() {
        let view = protocol_manifest_for("gemini", Chat);
        assert_eq!(view.protocols.len(), 3);
        assert_eq!(view.protocols[0].protocol_id, "gemini.generate_text");
        assert!(view.protocols[1..]
            .iter()
            .all(|descriptor| descriptor.default_connections.is_empty()));
        let recommendation = view.recommendation.unwrap();
        assert_eq!(recommendation.protocol_id, "gemini.generate_text");
        assert_eq!(
            recommendation.default_base_url.as_deref(),
            Some("https://generativelanguage.googleapis.com")
        );
        assert_eq!(
            recommendation.default_auth_scheme.as_deref(),
            Some("header_key:x-goog-api-key")
        );
    }

    #[test]
    fn every_preset_has_a_default_url_or_requires_user_input() {
        let mut ids = BTreeSet::new();
        for preset in platform_presets() {
            assert!(ids.insert(preset.preset.clone()), "duplicate preset {}", preset.preset);
            assert!(
                preset.platform_default_base_url.is_some() || preset.requires_user_input,
                "preset {} needs a default URL or requires_user_input",
                preset.preset
            );
        }
        assert!(ids.contains("SiliconFlow-CN"));
        assert!(ids.contains("SiliconFlow"));
        assert!(ids.contains("custom"));
        assert!(ids.contains("AWS-Bedrock"));
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

    fn joined_recommendation_url(
        view: &ModelProtocolManifestResponse,
        endpoint: &ProtocolEndpointDescriptor,
    ) -> String {
        let recommendation = view.recommendation.as_ref().expect("recommendation");
        let descriptor = protocol_task_descriptor(&recommendation.protocol_id, view.requested_task)
            .expect("recommended protocol descriptor");
        let base = recommendation.default_base_url.as_deref().expect("recommended base URL");
        // Compose through the production joiner, not a test-local copy: a
        // snapshot built by a second implementation would stay green even if
        // the real joiner regressed.
        let mut joined = crate::url_algebra::join_endpoint(base, &endpoint.default_value);
        if descriptor.transport == Websocket {
            if let Some(tail) = joined.strip_prefix("https://") {
                joined = format!("wss://{tail}");
            } else if let Some(tail) = joined.strip_prefix("http://") {
                joined = format!("ws://{tail}");
            }
        }
        joined
    }

    #[test]
    fn every_preset_recommendation_has_a_stable_exact_composed_url_snapshot() {
        let mut lines = Vec::new();
        for preset in platform_presets() {
            for task in ALL_MODEL_TASKS {
                let view = protocol_manifest_for(&preset.preset, task);
                if let Some(recommendation) = &view.recommendation {
                    let descriptor = protocol_task_descriptor(&recommendation.protocol_id, task)
                        .expect("recommended protocol descriptor");
                    if descriptor.transport == Sdk {
                        lines.push(format!(
                            "{}|{:?}|{}|sdk",
                            preset.preset, task, recommendation.protocol_id
                        ));
                        continue;
                    }
                    for endpoint in &descriptor.endpoints {
                        let url = joined_recommendation_url(&view, endpoint);
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
        }
        let snapshot = lines.join("\n");
        let hash = snapshot.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
        assert_eq!(
            hash, 9_446_312_405_170_401_367,
            "recommendation URL snapshot changed:\n{snapshot}"
        );
    }

    #[test]
    fn task_specific_connection_overrides_are_explicit() {
        let gemini_chat = protocol_manifest_for("gemini", Chat).recommendation.unwrap();
        assert!(!gemini_chat.base_url_override_required);
        let dashscope_image = protocol_manifest_for("Dashscope", ImageGeneration)
            .recommendation
            .unwrap();
        assert!(dashscope_image.base_url_override_required);
        assert_eq!(dashscope_image.default_base_url.as_deref(), Some("https://dashscope.aliyuncs.com"));

        let ark_voice = protocol_manifest_for("Ark", SpeechSynthesis).recommendation.unwrap();
        assert_eq!(ark_voice.connection_role.as_deref(), Some("voice"));
        assert!(!ark_voice.base_url_override_required);
        assert_eq!(ark_voice.default_auth_scheme.as_deref(), Some("volc_voice"));
    }

    #[test]
    fn stored_base_url_disambiguates_regional_presets_with_one_platform_id() {
        let cn = protocol_manifest_for_connection(
            "siliconflow",
            Some("https://api.siliconflow.cn/v1/"),
            Chat,
        );
        assert_eq!(cn.preset, "SiliconFlow-CN");
        assert_eq!(
            cn.platform_default_base_url.as_deref(),
            Some("https://api.siliconflow.cn/v1")
        );
        assert!(cn.protocols.iter().all(|protocol| {
            protocol
                .default_connections
                .iter()
                .all(|connection| connection.preset == "SiliconFlow-CN")
        }));

        let global = protocol_manifest_for_connection(
            "siliconflow",
            Some("https://api.siliconflow.com/v1"),
            Chat,
        );
        assert_eq!(global.preset, "SiliconFlow");
    }

    #[test]
    fn stepfun_plan_defaults_cover_images_tts_and_realtime() {
        for (task, protocol) in [
            (ImageGeneration, "stepfun.images"),
            (SpeechSynthesis, "stepfun.audio_speech"),
            (RealtimeConversation, "stepfun.realtime_s2s"),
        ] {
            let view = protocol_manifest_for("StepFun-Plan", task);
            let recommendation = view.recommendation.expect("Step Plan task recommendation");
            assert_eq!(recommendation.protocol_id, protocol);
            assert_eq!(
                recommendation.default_base_url.as_deref(),
                Some("https://api.stepfun.com/step_plan/v1")
            );
        }
    }
}
