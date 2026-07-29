# P1 统一调用层（nomifun-model-invoke）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 新建底层基础 crate `nomifun-model-invoke`（统一多模态任务调用：typed 请求 → 协议适配器注册表 → 供应商 API），把 creation 的适配器与 STT 双实现迁入，探针与真实调用走同一管线，落地火山（ark 图像/视频 + openspeech ASR 多连接档案）与 TTS 端到端，铲除全部"按名字猜协议"路由。

**Architecture:** 依赖方向 = 产品（creation/shell/ai-agent probe）→ invoke → (common/api-types/db/net)。适配器接口 `ProtocolAdapter`（submit/poll，`(protocol, task)` 显式注册表路由）；目录解析链 = provider 行 + provider_models 行 + provider_connections 行 + 平台路由表 → `ResolvedCall`；错误统一 `InvokeError`。chat 链路本期不动（P1 偏差：`map_nomi_provider`/`resolve_nomi_url_and_compat` 的表化统一推迟到 P2，记录在案）。

**Tech Stack:** Rust (async-trait + reqwest + wiremock)；base 分支 `dev/model-catalog-p1-20260729`（stacked on P0 195f23c4）。

## Global Constraints

- 依赖红线：`nomifun-model-invoke` 只依赖 nomifun-common / nomifun-api-types / nomifun-db / nomifun-net + 通用库；**禁止**依赖任何产品功能 crate。
- 路由纪律：适配器查找键只允许 `(protocol_id, ModelTask)`；模型名子串/平台名 if-else 路由一律禁止（名称启发式只允许存在于 api-types 的播种函数）。
- wire 兼容：`/api/creation/tasks`、`/api/stt`、`/api/agents/provider-health-check` 请求/响应形状不变；creation 现有 wiremock e2e 测试（`service.rs::http_e2e_tests`）必须不改断言通过（它们是适配器迁移的行为快照）。
- creation 的 `MediaCapability`/`CreationError` 保留在 creation；映射 `MediaCapability→TaskRequest`、`InvokeError→CreationError` 在 creation 侧。
- 每任务测试：`cargo test -p nomifun-model-invoke`（新）+ 受改 crate 套件 + `cargo check --workspace --exclude nomifun-desktop`；提交信息结尾 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`；禁止触碰 .github/workflows。
- 既有失败基线：36 个 workspace 既有失败（见 P0 handoff §验证记录），与本分支无关，不要追逐。

## File Structure

- Create: `crates/backend/nomifun-model-invoke/`（Cargo.toml, src/{lib.rs, error.rs, types.rs, auth.rs, transport.rs, adapter.rs, routes_table.rs, resolve.rs, service.rs, adapters/{mod.rs, openai_images.rs, openai_videos.rs, openai_audio.rs, openai_embeddings.rs, openai_chat_text.rs, gemini.rs, deepgram.rs, ark.rs, volc_voice.rs}}）
- Modify: 根 Cargo.toml `[workspace.dependencies]` 加 `nomifun-model-invoke = { path = ... }`
- Modify: `nomifun-creation`（service.rs/builder/lib.rs 删 adapters+provider.rs，接 invoke）、`nomifun-shell`（routes.rs/stt.rs/state.rs + 新 tts 路由）、`nomifun-ai-agent/src/services/provider_health.rs`（modality probe 委托 invoke）、`nomifun-app`（services.rs 装配 + router/state.rs）
- Delete (T6): `nomifun-creation/src/adapters/*`、`provider.rs`；(T7): `nomifun-shell/src/stt_openai.rs`、`stt_deepgram.rs`

---

### Task 1: crate 骨架 —— 类型 / 错误 / 鉴权 / 传输助手 / 注册表 / 平台路由表

**Files:** Create crate（见 File Structure），根 Cargo.toml 登记。Cargo.toml 依赖：nomifun-common/api-types/db/net + serde/serde_json/async-trait/reqwest/tokio/tracing/base64/thiserror（workspace 版）；dev: wiremock/tempfile。

**Interfaces (后续任务 verbatim 消费):**

```rust
// error.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InvokeErrorKind { Auth, RateLimited, QuotaExhausted, UnsupportedTask, NoAdapter,
    MissingConnection, InvalidParams, ContentPolicy, ProviderError, Network, Timeout,
    ParseError, NotPollable, Config }
#[derive(Debug, Clone, thiserror::Error)]
#[error("{kind:?}: {message}")]
pub struct InvokeError { pub kind: InvokeErrorKind, pub message: String,
    pub http_status: Option<u16>, pub retry_after_ms: Option<u64> }
impl InvokeError {
    pub fn new(kind: InvokeErrorKind, message: impl Into<String>) -> Self; // http_status/retry None
    pub fn provider(status: u16, message: impl Into<String>) -> Self;     // kind=ProviderError + status
    pub fn config(msg: impl Into<String>) -> Self; pub fn network(e: &reqwest::Error) -> Self; // is_timeout→Timeout else Network
    pub fn parse(msg: impl Into<String>) -> Self; pub fn not_pollable() -> Self;
}

// types.rs
#[derive(Debug, Clone)] pub struct ModelRef { pub provider_id: String, pub model: String }
#[derive(Clone)] pub struct InputAsset { pub id: Option<String>, pub role: String, pub bytes: Vec<u8>, pub mime: String } // no Debug (bytes)
#[derive(Clone)] pub struct ImageGenRequest { pub prompt: String, pub count: u32, pub size: Option<String>, pub quality: Option<String>, pub extra: serde_json::Value }
#[derive(Clone)] pub struct ImageEditRequest { pub prompt: String, pub count: u32, pub size: Option<String>, pub inputs: Vec<InputAsset>, pub extra: serde_json::Value } // mask = role=="mask"
#[derive(Clone)] pub struct VideoGenRequest { pub prompt: String, pub seconds: Option<u32>, pub size: Option<String>, pub inputs: Vec<InputAsset>, pub extra: serde_json::Value }
#[derive(Clone)] pub struct TtsRequest { pub text: String, pub voice: Option<String>, pub format: Option<String>, pub extra: serde_json::Value }
#[derive(Clone)] pub struct AsrRequest { pub audio: InputAsset, pub language: Option<String>, pub prompt: Option<String>, pub extra: serde_json::Value }
#[derive(Clone)] pub struct EmbedRequest { pub inputs: Vec<String>, pub extra: serde_json::Value }
#[derive(Clone)] pub struct ChatTextRequest { pub prompt: String, pub system: Option<String>, pub extra: serde_json::Value }
#[derive(Clone)] pub enum TaskRequest { ImageGeneration(ImageGenRequest), ImageEdit(ImageEditRequest),
    VideoGeneration(VideoGenRequest), SpeechSynthesis(TtsRequest), SpeechRecognition(AsrRequest),
    Embedding(EmbedRequest), ChatText(ChatTextRequest) }
impl TaskRequest { pub fn task(&self) -> nomifun_api_types::ModelTask } // ChatText→Chat
#[derive(Debug, Clone)] pub enum ProducedData { Bytes(Vec<u8>), Url(String) }
#[derive(Debug, Clone)] pub struct ProducedAsset { pub data: ProducedData, pub mime: Option<String> }
#[derive(Debug, Clone)] pub enum TaskResult { Assets(Vec<ProducedAsset>),
    Transcript { text: String, language: Option<String>, model: Option<String> },
    Embeddings(Vec<Vec<f32>>), Text(String) }
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)] pub struct JobHandle { pub adapter_id: String, pub remote_id: String, #[serde(default)] pub poll_state: serde_json::Value }
#[derive(Debug, Clone)] pub enum TaskOutcome { Done(TaskResult), Pending(JobHandle) } // Debug ok: assets are Debug

// auth.rs — 声明式鉴权；凭证 JSON 形状由 scheme 决定
#[derive(Debug, Clone, PartialEq)] pub enum AuthScheme { Bearer, TokenHeader,
    HeaderKey(String), MultiHeader(Vec<(String, String)>), QueryKey(String) }
impl AuthScheme { pub fn parse(s: &str) -> Result<Self, InvokeError> }
// "bearer" | "token" | "header_key:<name>" | "query_key:<param>" | "volc_voice"（内置别名 → MultiHeader 模板见 T9）
#[derive(Clone)] pub struct AuthMaterial { pub scheme: AuthScheme, pub credentials: serde_json::Value } // no Debug
impl AuthMaterial {
    /// bearer/token/header_key/query_key 取 credentials["api_keys"][0]（或兼容裸 {"api_key": "..."}）；
    /// providers 行 default 连接：credentials = {"api_keys": [第一个逗号/换行分隔 key]}
    pub fn primary_secret(&self) -> Result<String, InvokeError>;
    pub fn apply(&self, rb: reqwest::RequestBuilder) -> Result<reqwest::RequestBuilder, InvokeError>;
}

// transport.rs — 从 creation adapters/mod.rs 平移（原文见该文件）：
pub fn net_err(e: reqwest::Error) -> InvokeError;                       // 原 net_err，映射 InvokeError
pub async fn error_from_response(resp: reqwest::Response) -> InvokeError; // 状态+截 500 字符 body；429→RateLimited(+Retry-After 头解析)；401/403→Auth；400/422→InvalidParams；5xx→ProviderError
pub const MAX_ARTIFACT_BYTES: u64 = 256 * 1024 * 1024;
pub async fn read_body_capped(resp: reqwest::Response, max: u64) -> Result<Vec<u8>, InvokeError>;
pub fn decode_b64(s: &str) -> Option<Vec<u8>>; pub fn encode_b64(b: &[u8]) -> String;

// call.rs（放 types.rs 亦可）
#[derive(Clone)] pub struct ResolvedConnection { pub role: String, pub base_url: String, pub is_full_url: bool, pub auth: AuthMaterial, pub extra: serde_json::Value }
#[derive(Clone)] pub struct ResolvedCall { pub provider_id: String, pub platform: String, pub model: String,
    pub task: nomifun_api_types::ModelTask, pub connection: ResolvedConnection,
    pub model_params: serde_json::Value, pub request: TaskRequest }
impl ResolvedCall { pub fn dispatch_target(&self) -> nomifun_api_types::DispatchTarget {
    nomifun_api_types::resolve_dispatch_target(&self.platform, &self.connection.base_url,
        self.connection.is_full_url, self.task, &self.model_params) } }

// adapter.rs
#[async_trait::async_trait] pub trait ProtocolAdapter: Send + Sync {
    fn id(&self) -> &'static str;
    fn supports(&self, task: nomifun_api_types::ModelTask) -> bool;
    async fn submit(&self, http: &reqwest::Client, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError>;
    async fn poll(&self, _http: &reqwest::Client, _call: &ResolvedCall, _job: &JobHandle) -> Result<TaskOutcome, InvokeError> { Err(InvokeError::not_pollable()) }
}
pub struct AdapterRegistry { map: std::collections::HashMap<&'static str, std::sync::Arc<dyn ProtocolAdapter>> }
impl AdapterRegistry {
    pub fn new(adapters: Vec<std::sync::Arc<dyn ProtocolAdapter>>) -> Self; // key = adapter.id()
    pub fn get(&self, protocol: &str, task: nomifun_api_types::ModelTask) -> Result<std::sync::Arc<dyn ProtocolAdapter>, InvokeError>;
    // 未注册 → NoAdapter{message 含 protocol+task}；注册但 !supports(task) → NoAdapter
}

// routes_table.rs — 平台路由表（P1 内置常量表；铲平台特判的单点）
pub struct TaskRoute { pub protocol: &'static str, pub connection_role: Option<&'static str> }
pub fn platform_route(platform: &str, task: ModelTask) -> TaskRoute;
// 默认（任意平台）：Chat→"openai.chat_text"; ImageGeneration|ImageEdit→"openai.images";
//   VideoGeneration→"openai.videos"; SpeechSynthesis→"openai.audio_speech";
//   SpeechRecognition→"openai.audio_transcriptions"; Embedding→"openai.embeddings"; Rerank→"openai.rerank"
// 覆盖: platform=="gemini" → ImageGeneration|ImageEdit→"gemini.generate_content", Chat→"gemini.generate_text"
//       platform=="deepgram" → SpeechRecognition→"deepgram.listen"
//       platform=="ark"|"volcengine" → ImageGeneration→"ark.images", VideoGeneration→"ark.video_jobs",
//           SpeechRecognition→TaskRoute{"volc.asr_file", Some("voice")}, SpeechSynthesis→TaskRoute{"volc.tts_v3", Some("voice")}
// connection_role: None = default 连接（providers 行）
```

- [ ] **Step 1:** 失败单测（error 构造映射、AuthScheme::parse 全词表、AuthMaterial::apply 对 4 种 scheme 的 header/query 注入（用 reqwest Request 构建后断言 header）、primary_secret 兼容 {"api_keys":[..]} 与 {"api_key":".."}、registry get 命中/NoAdapter 两分支（用 10 行 FakeAdapter）、platform_route 全矩阵表驱动、read_body_capped 超限（wiremock，平移 creation 现有测试）、error_from_response 429→RateLimited+retry_after / 401→Auth / 400→InvalidParams / 500→ProviderError）
- [ ] **Step 2-4:** 红 → 实现 → `cargo test -p nomifun-model-invoke` 绿 + workspace check
- [ ] **Step 5:** Commit `feat(model-invoke): scaffold unified invocation crate (types, errors, auth schemes, adapter registry, platform routes)`

---

### Task 2: 目录解析管线 —— ResolvedCall 构建与守门

**Files:** Create `src/resolve.rs`、`src/service.rs`（构造器+resolve 部分）；Modify lib.rs。

**Interfaces:**

```rust
pub struct ModelInvokeService {
    provider_repo: std::sync::Arc<dyn nomifun_db::IProviderRepository>,
    provider_model_repo: std::sync::Arc<dyn nomifun_db::IProviderModelRepository>,
    provider_connection_repo: std::sync::Arc<dyn nomifun_db::IProviderConnectionRepository>,
    encryption_key: [u8; 32],
    http: reqwest::Client,
    registry: AdapterRegistry,
}
impl ModelInvokeService {
    pub fn new(provider_repo, provider_model_repo, provider_connection_repo, encryption_key, http, registry) -> Self;
    /// enforce_task_membership=false 用于探针（显式 task 探测未打标模型）
    pub(crate) async fn resolve(&self, m: &ModelRef, task: ModelTask, request: TaskRequest,
        enforce_task_membership: bool) -> Result<(ResolvedCall, std::sync::Arc<dyn ProtocolAdapter>), InvokeError>;
}
```

resolve 规则（唯一算法，注释写明）：
1. `ProviderId::parse` → InvalidParams；`provider_repo.find_by_id` → 无 → Config("provider not found")；`!provider.enabled` → Config("provider disabled")。
2. `provider_model_repo.get(provider_id, model)`：无行 → enforce 时 `UnsupportedTask`("model not in catalog")，探针时容忍（tasks 视为空）；行 disabled → `UnsupportedTask`("model disabled")。
3. enforce 时：解析行 tasks JSON（坏 JSON → 空）→ 若 task ∉ tasks 且 tasks 非空 → `UnsupportedTask`；tasks 为空（未播种）→ 回退 `derive_tasks_and_traits(platform, model)` 判定，仍不含 → `UnsupportedTask`。
4. protocol = 行.protocol（非空） ?? `platform_route(platform, task).protocol`；role = 行.connection_role ?? route.connection_role。
5. 连接：role None → default：`ResolvedConnection { role: "default", base_url: provider.base_url, is_full_url: provider.is_full_url, auth: AuthMaterial { scheme: Bearer, credentials: {"api_keys":[decrypt(api_key_encrypted) 按 [',','\n'] split 取第一个非空]} }, extra: {} }`（bedrock 平台在 P1 不经 invoke——`platform=="bedrock"` 且走 default 连接时返回 Config("bedrock is not supported by the invoke layer yet")）。role Some(r) → `provider_connection_repo.get(provider_id, r)`：无 → `MissingConnection`(message 含 role 与"请在供应商连接档案中配置")；有 → decrypt credentials + `AuthScheme::parse(auth_scheme)`。
6. model_params = 行.params JSON（坏 → {}）；`registry.get(protocol, task)`。

- [ ] **Step 1:** 失败测试（`init_database_memory` + 真仓储 + FakeAdapter 注册表）：happy path default 连接（断言 base_url/密钥解密/协议默认 openai.*）；行 protocol 覆盖生效；connection_role 缺档案 → MissingConnection；task 不在 tasks → UnsupportedTask；未播种行 + 名称可推断 → 放行（模型名 "gpt-image-1" task=ImageGeneration）；探针模式跳过 membership；provider/model disabled 两分支。
- [ ] **Step 2-4:** 红 → 实现 → 绿 + workspace check
- [ ] **Step 5:** Commit `feat(model-invoke): catalog resolution pipeline with connection profiles and task gating`

---

### Task 3: OpenAI 族适配器移植（images / videos / chat_text / embeddings）

**Files:** Create `src/adapters/{mod.rs, openai_images.rs, openai_videos.rs, openai_chat_text.rs, openai_embeddings.rs}`。移植源（P0 前全文已在 creation crate，未被 P0 改动）：`nomifun-creation/src/adapters/{openai_images.rs, openai_video.rs, openai_chat.rs}`。

要点：
- `openai.images`（supports ImageGeneration+ImageEdit）：generations = `call.dispatch_target()` URL + JSON `{model, prompt, n, response_format:"b64_json", size?, quality?}`（字段取自 `TaskRequest::ImageGeneration`；`quality` 从 typed 字段而非 extra）；edits = multipart（单图 `image`/多图 `image[]`、`mask`）；响应解析 `parse_images_response`（b64 优先/url 兜底）原样平移含其单测。鉴权改 `call.connection.auth.apply(rb)`（替代硬编码 Bearer）。
- `openai.videos`（VideoGeneration）：**改走 `call.dispatch_target()`**（修复 P0 已知问题：`params.endpoint` 覆盖对视频无效）——submit multipart（model/prompt/seconds/size + `input_reference` 首帧）→ `TaskOutcome::Pending(JobHandle{adapter_id:"openai.videos", remote_id:id, poll_state:{}})`；poll：`GET {video_base}/{id}` 状态词表宽容匹配（completed/succeeded/done→下载 `/{id}/content` 字节→Done(Assets)；failed/error→ProviderError；其余→Pending）。video_base 推导：dispatch_target 的 URL（去 query）即 submit URL；poll 直接拼 `{submit_url}/{id}`。
- `openai.chat_text`（Chat）：非流式 `/chat/completions`，`ChatTextRequest{prompt, system}` → messages；响应取 `choices[0].message.content` → `TaskResult::Text`。
- `openai.embeddings`（Embedding，新写，小）：JSON `{model, input: [..]}` → `data[].embedding` → `TaskResult::Embeddings`。
- mod.rs：`pub fn default_adapters() -> Vec<Arc<dyn ProtocolAdapter>>`（本任务先含这 4 个，后续任务追加）。

- [ ] **Step 1:** wiremock 失败测试逐适配器：images gen（b64 响应→Bytes）、images edit（multipart 收到 `image[]` 字段名——用 wiremock 请求体断言）、videos submit→poll→content 三段（平移 creation e2e 的 mock 编排）、`params.endpoint` 覆盖对 videos 生效（mock 非常规路径）、chat_text、embeddings；401 → InvokeErrorKind::Auth。
- [ ] **Step 2-4:** 红 → 实现 → 绿
- [ ] **Step 5:** Commit `feat(model-invoke): port OpenAI-family adapters (images, videos, chat text, embeddings)`

---

### Task 4: gemini / deepgram / openai.audio_transcriptions 适配器

**Files:** Create `src/adapters/{gemini.rs, deepgram.rs, openai_audio.rs 的 transcriptions 部分}`。移植源：`nomifun-creation/src/adapters/{gemini_image.rs, gemini_text.rs}`、`nomifun-shell/src/{stt_openai.rs, stt_deepgram.rs}`（全文在各文件，P0 未动）。

要点：
- `gemini.generate_content`（ImageGeneration+ImageEdit）：URL `{root}/v1beta/models/{model}:generateContent`（容忍尾部 /v1beta，平移 `gemini_generate_url`），鉴权 = `call.connection.auth.apply`（连接 auth_scheme 应为 `header_key:x-goog-api-key`；default 连接是 Bearer 时**仍用 header_key**——gemini 平台在 routes_table 有覆盖，且 apply 由 AuthMaterial 决定：对 gemini 平台 default 连接，resolve 阶段将 scheme 改写为 HeaderKey("x-goog-api-key")——在 T2 的 resolve 第 5 步加一行平台 default-auth 覆盖表：gemini→header_key:x-goog-api-key，deepgram→token）；body contents.parts（text + inline_data）+ `responseModalities:["TEXT","IMAGE"]`；解析 inlineData（camel/snake 双容忍）；**count>1 循环请求**（补 P0 已知缺陷：n 不被 gemini 支持，循环 count 次聚合 assets，任一失败即失败）。`gemini.generate_text`（Chat）同文件。
- `deepgram.listen`（SpeechRecognition）：`{base}/v1/listen` + query（model/language|detect_language/punctuate/smart_format 从 AsrRequest+extra）；裸二进制 body + Content-Type=audio mime；鉴权 Token 前缀；解析 transcript/model/detected_language 平移 stt_deepgram 全部逻辑与单测。
- `openai.audio_transcriptions`（SpeechRecognition）：dispatch_target（Multipart）+ file/model/response_format=json/language/prompt/temperature(extra)；解析 `{text}` → Transcript。

- [ ] **Step 1:** wiremock 失败测试：gemini 图像 b64 解析 + count=2 循环两次请求（wiremock expect(2)）+ x-goog-api-key 头断言；deepgram query 参数与 Token 头断言 + transcript 解析（平移原单测数据）；transcriptions multipart 字段断言 + StepFun 风格 `{base}/v1` 归一。
- [ ] **Step 2-4:** 红 → 实现 → 绿
- [ ] **Step 5:** Commit `feat(model-invoke): gemini, deepgram and openai transcription adapters`

---

### Task 5: invoke / poll / probe 编排

**Files:** Modify `src/service.rs`。

**Interfaces:**

```rust
impl ModelInvokeService {
    pub async fn invoke(&self, m: &ModelRef, req: TaskRequest) -> Result<TaskOutcome, InvokeError>;
    // = resolve(enforce=true) → adapter.submit(http, call)
    pub async fn poll(&self, m: &ModelRef, req: TaskRequest, job: &JobHandle) -> Result<TaskOutcome, InvokeError>;
    // = resolve(enforce=false)（恢复场景模型可能已改标签，轮询不重新守门）→ registry 按 job.adapter_id 直取 → adapter.poll
    pub async fn probe(&self, m: &ModelRef, task: ModelTask) -> Result<ProbeReport, InvokeError>;
}
#[derive(Debug, Clone)] pub struct ProbeReport { pub healthy: bool, pub latency_ms: u64, pub message: Option<String> }
```

probe 规则（平移 `provider_health.rs::run_modality_probe`+`minimal_json_body` 语义）：按 task 构造最小 TaskRequest（ImageGeneration: prompt "health check"/count 1/size 取 model_params.size；SpeechSynthesis: text "hi"/voice 取 params.voice 或 "alloy"；Embedding: ["health check"]；SpeechRecognition/ImageEdit: 空文件 multipart——adapter.submit 会因缺文件收到 4xx，**InvalidParams 视为 healthy**（reachable-only，语义与现探针一致）；VideoGeneration: prompt only，收到 Pending 即 healthy（不继续轮询，不下载）；Chat 不支持 → Config 错误（chat 探针留在 ai-agent 引擎路径））。60s 超时（`tokio::time::timeout`）。成功/宽容分支 → healthy + latency；InvokeError 其余 → healthy=false + message。

- [ ] **Step 1:** 失败测试（真 DB + wiremock + default_adapters）：invoke 图像端到端（种 provider+模型行 tasks=["image_generation"]）；task 不符 → UnsupportedTask；probe 对 multipart 任务收 400 invalid_request 仍 healthy；probe 对 500 → unhealthy；video probe 收 Pending → healthy 且 wiremock 只收到 submit 无 poll。
- [ ] **Step 2-4:** 红 → 实现 → 绿
- [ ] **Step 5:** Commit `feat(model-invoke): invoke/poll/probe orchestration sharing one resolution pipeline`

---

### Task 6: creation 切换到 invoke（删除内嵌适配器）

**Files:** Modify `nomifun-creation/{Cargo.toml(+nomifun-model-invoke), src/{service.rs, lib.rs, types.rs}}`；Delete `src/adapters/*`、`src/provider.rs`；Modify `nomifun-app/src/services.rs`（装配：先构造 `ModelInvokeService`（Arc）——`default_adapters()` 来自 invoke——注入 creation builder `.with_invoke(invoke.clone())`，删除 `.with_providers(...)/.with_provider_repo(...)`；`nomifun_creation::default_adapters` 引用删除）；Modify `nomifun-gateway`（若引用被删 re-export 则调整——它只用 CreationInput/NewCreationTask/CreationService，应无需改）。

变更规范：
- `CreationServiceBuilder::with_invoke(Arc<ModelInvokeService>)` 替代 with_providers/with_provider_repo；`CreationService.execute`：`resolve_provider`/`select_adapter` 删除，改为 `cap_to_task_request(job.capability, &job.params, inputs) -> Result<TaskRequest, CreationError>`（映射表：T2i→ImageGeneration{prompt=param_prompt, count=param_count, size=param_size, quality=params.quality}；I2i|Inpaint→ImageEdit（mask 进 inputs role="mask"）；T2v|I2v→VideoGeneration{seconds=params.seconds, size=param_size}；V2v→CreationError::new("unsupported_capability",...)（与现状一致：openai_video 不支持 v2v）；Tts→SpeechSynthesis{text=param_prompt, voice=params.voice}（**Tts 由此从"必失败"变为可用**——路由到 openai.audio_speech，T8 落地后端到端）；Text→ChatText{prompt=param_prompt}）→ `invoke.invoke(&ModelRef{provider_id: job.provider_id, model: job.model}, req)`。
- `param_prompt/param_count/param_size` 留在 creation（service.rs 顶部私有 fn，从旧 adapters/mod.rs 平移——count 校验语义不变 invalid_params）。
- Pending 分支：`TaskOutcome::Pending(job)` → 持久化 `remote_task_id = serde_json::to_string(&job)`（**兼容旧行**：读取时先尝试 JSON 反解 JobHandle，失败则视为旧格式裸 id → `JobHandle{adapter_id: 按 capability 映射默认协议, remote_id: 原串, poll_state:{}}`——boot resume 兼容）；poll_loop 调 `invoke.poll(...)`，`InvokeError` → `CreationError`（`impl From<InvokeError> for CreationError`：kind 映射 provider_error/timeout/invalid_params/unsupported_capability/config + http_status 透传；4xx 终态判断沿用 http_status）。
- `TaskResult::Assets` → 现有 ProducedAsset 持久化管线（invoke 的 ProducedAsset 与 creation 原类型同构——creation 内部改用 invoke 的类型，artifact.rs 的 `validate_for_capability` 签名同步）；`TaskResult::Text` → 现有 text 产物路径（text/plain bytes）。
- **守门收紧（计划内新行为）**：invoke 的 UnsupportedTask 映射 CreationError kind="unsupported_capability" —— 创建任务时选了没打标的模型会得到 typed 错误而非打错端点。creation 的 e2e 测试种子模型行需相应打标（`build()` 里 provider 创建后为测试模型 upsert provider_models 行 tasks 对应 capability——用 `SqliteProviderModelRepository` 直接写）。
- lib.rs：删除 `default_adapters/route_adapter_id/MediaProvider/...` re-export（workspace grep 确认无外部引用后删；`ResolvedProvider` 等同删）。

- [ ] **Step 1:** 先跑 `cargo test -p nomifun-creation` 记录基线（全绿）→ 动刀 → e2e 测试（openai_images_end_to_end / openai_video 全链 / openai_chat / gemini_text / 401 传播 / URL 下载校验族）**不改断言**恢复全绿（种子打标除外——那是 setup 不是断言）；新增：Tts capability 现在路由成功（wiremock /v1/audio/speech 返回字节 → 任务 succeeded 产物 audio/mpeg——依赖 T8 的 openai.audio_speech：**本任务与 T8 顺序不可换**，若先做本任务则 Tts 测试标 ignore 并在 T8 解除）。→ 调整任务顺序：**T8（audio_speech + /api/tts）提前到本任务之前执行**（编号保持，执行顺序 1,2,3,4,5,8,6,7,9,10）。
- [ ] **Step 2:** `cargo test -p nomifun-creation -p nomifun-app -p nomifun-gateway` + workspace check 绿
- [ ] **Step 3:** Commit `refactor(creation): consume nomifun-model-invoke, delete embedded adapters and provider resolution`

---

### Task 7: shell STT + 探针切换

**Files:** Modify `nomifun-shell/{Cargo.toml(+invoke), src/{routes.rs, stt.rs, state.rs}}`；Delete `stt_openai.rs`、`stt_deepgram.rs`；Modify `nomifun-ai-agent/{Cargo.toml(+invoke), src/services/provider_health.rs}`；Modify `nomifun-app/src/router/state.rs`（两处 state 装配注入 `Arc<ModelInvokeService>`）。

- STT：`resolve_cloud_speech_to_text_config` 保留偏好读取与校验，但 provider_id 分支不再展开成 OpenAI/Deepgram 双 config——改为 `invoke.invoke(&ModelRef{provider_id, model}, SpeechRecognition(AsrRequest{audio, language, ..}))`；**协议由平台路由表决定**（platform=="deepgram"→deepgram.listen，其余→openai.audio_transcriptions，模型行 protocol 可覆盖）——前端的 provider 枚举猜测被忽略（wire 兼容：字段仍接受）。legacy 内嵌 key 模式（无 provider_id、config.openai/deepgram 直填）保留：把内嵌 config 构造成一次性 ResolvedCall 太绕——**保留旧两个文件？** 否：legacy 模式在 `SpeechToTextConfig` 反序列化即有（`openai:/deepgram:` 字段）；grep 前端确认现 UI 只写 provider_id 模式 → legacy 分支改为返回 `SttError::Unknown("embedded-credential speech config is no longer supported; select a provider in settings")`（**wire 行为变化，记录在 handoff**；若执行时 grep 发现 UI 仍会发 legacy 形态则改回保留旧实现文件并缩小本条为"仅 provider_id 分支切 invoke"——执行者验证后二选一，报告说明）。`SpeechToTextResult` 从 `TaskResult::Transcript` 组装（provider 字段按协议回填 openai/deepgram 枚举——从 adapter id 推断）。
- 探针：`provider_health.rs` 非 chat 分支（`run_modality_probe` 及 `minimal_json_body/minimal_multipart_form`）删除，改 `invoke.probe(&ModelRef{...}, task)` → 组装 `ProviderHealthCheckResponse`（healthy/latency/message 映射现有字段；`classify_error` 对 message 继续分类）；`persist_probe_outcome` 不动；chat 分支不动。`ProviderHealthCheckService::new` 增参 `invoke: Arc<ModelInvokeService>`。
- 现有 `provider_health.rs` 单测中直接调 `run_modality_probe` 的（955/987 行附近）改为经 service 或迁至 invoke::probe 的测试（语义等价：multipart 400 仍 healthy 已在 T5 测）。

- [ ] **Step 1:** 失败测试：shell route 层（wiremock 供应商 + 真 DB：deepgram 平台走 Token 头（wiremock 断言）、openai 平台走 multipart；模型未启用→错误不变）；ai-agent `cargo test -p nomifun-ai-agent`（除既有 openclaw 失败）绿。
- [ ] **Step 2:** workspace check + 受改 crate 套件绿
- [ ] **Step 3:** Commit `refactor(shell,ai-agent): route STT and modality probes through nomifun-model-invoke`

---

### Task 8: openai.audio_speech 适配器 + POST /api/tts（执行顺序在 T6 前）

**Files:** Create `nomifun-model-invoke/src/adapters/openai_audio.rs` 的 speech 部分（`openai.audio_speech`，supports SpeechSynthesis）：dispatch_target（Json，约定路径 /audio/speech）+ body `{model, input, voice(默认 "alloy"), response_format?}` → 响应**裸二进制** `read_body_capped` → `TaskResult::Assets([ProducedAsset{Bytes, mime: format→mime 映射（mp3→audio/mpeg, wav→audio/wav, opus→audio/ogg, aac→audio/aac, flac→audio/flac；None→响应 Content-Type 头，再兜底 audio/mpeg)}])`。注册进 `default_adapters()`。
Modify `nomifun-shell/src/routes.rs`：`POST /api/tts`，body：

```rust
#[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct TtsApiRequest {
    provider_id: String, model: String, text: String,
    #[serde(default)] voice: Option<String>, #[serde(default)] format: Option<String> }
```

（放 nomifun-api-types/src/shell.rs，provider_id/model 走 serde_util 校验）→ `invoke.invoke(.., SpeechSynthesis(TtsRequest{..}))` → 响应：`Response` 直接回音频字节 + Content-Type（非 ApiResponse 包络——二进制端点，同 office 预览先例）；错误 → `InvokeError` 映射 `AppError`（`impl From<InvokeError> for AppError` 放 invoke crate？不行，依赖方向 common←invoke 可以：AppError 在 common，invoke 依赖 common → 在 invoke 提供 `impl From<InvokeError> for nomifun_common::AppError`：Auth/ProviderError→BadGateway、UnsupportedTask/InvalidParams/MissingConnection/Config→BadRequest、Timeout→Timeout、RateLimited→RateLimited、其余→Internal）。文本上限 4096 字符（BadRequest）。

- [ ] **Step 1:** 失败测试：适配器 wiremock（body 断言 + 二进制回包 + mime 推断）；route 测试（真 DB 种 provider+模型行 tasks=["speech_synthesis"] + wiremock → 200 audio/mpeg 字节；未打标模型 → 400；文本超限 → 400）。
- [ ] **Step 2-4:** 红 → 实现 → 绿 → Commit `feat(model-invoke,shell): TTS adapter and POST /api/tts endpoint`

---

### Task 9: ark 适配器（火山方舟图像 + 视频异步任务）

**Files:** Create `nomifun-model-invoke/src/adapters/ark.rs`（两个适配器）+ 注册。协议依据 `docs/specs/2026-07-28-provider-protocol-variance.zh.md` §3（实现前 Read 该节）：
- `ark.images`（ImageGeneration）：`POST {root}/api/v3/images/generations`（base_url 已含或不含 /api/v3 都容忍：strip 尾部 "/api/v3" 再补——镜像 openai_versioned_base 手法；`params.endpoint` 覆盖优先——即先走 dispatch_target，若 URL 是约定式 /v1/images/generations 形态则替换为 ark 路径：**实现为：ark 适配器不使用 conventional 路径，自行组 URL，但 `model_params.endpoint` 存在时用 dispatch_target 的覆盖结果**）；body `{model, prompt, size?, response_format:"b64_json", watermark: extra.watermark?, seed/guidance_scale 从 extra 透传白名单}`；响应 `data[].url|b64_json` 复用 `parse_images_response`。
- `ark.video_jobs`（VideoGeneration）：submit `POST {root}/api/v3/contents/generations/tasks` body `{model, content:[{type:"text", text: prompt + 参数后缀}]}`——参数后缀 = `--resolution {size?} --duration {seconds?}`（仅当存在；variance 文档记载的 prompt 内参数编码）+ 图生视频时 content 追加 `{type:"image_url", image_url:{url:"data:{mime};base64,{b64}"}}`；响应 `{id}` → Pending。poll `GET .../tasks/{id}`：status `queued|running→Pending`、`succeeded→content.video_url→ProducedAsset::Url`、`failed|cancelled→ProviderError(message=error 字段)`。
- routes_table 的 ark 条目已在 T1 就位。

- [ ] **Step 1:** wiremock 失败测试：images（URL 命中 /api/v3/images/generations + b64 解析）；video submit（body content 文本含 --resolution 断言）→ poll running → poll succeeded（video_url → Url 资产）；failed → ProviderError；base_url 带 /api/v3 尾缀归一。
- [ ] **Step 2-4:** 红 → 实现 → 绿 → Commit `feat(model-invoke): Volcengine Ark image and async video adapters`

---

### Task 10: volc.asr_file 适配器（openspeech 多连接档案端到端验证件）

**Files:** Create `nomifun-model-invoke/src/adapters/volc_voice.rs`（`volc.asr_file`，SpeechRecognition）+ 注册 + T1 路由表 volcengine/ark 条目已指向 role "voice"。协议依据 variance 文档 §3 语音域（实现前 Read）：
- 鉴权 scheme `volc_voice`（T1 的 AuthScheme::parse 别名）：credentials JSON `{"app_key": "...", "access_key": "...", "resource_id": "volc.bigasr.auc"}` → MultiHeader `[("X-Api-App-Key",app_key),("X-Api-Access-Key",access_key),("X-Api-Resource-Id",resource_id)]`；每请求另加 `X-Api-Request-Id`（客户端 UUIDv7 生成——JobHandle.remote_id 即它，submit/query 复用，poll_state 存 `{"request_id": ...}`）。
- submit `POST {base}/api/v3/auc/bigmodel/submit` body `{user:{uid:"nomifun"}, audio:{format: mime→ext, data: base64}, request:{model_name: model}}` → 响应头 `X-Api-Status-Code`=="20000000" → Pending(JobHandle)。
- poll `POST {base}/api/v3/auc/bigmodel/query` body `{}`+同 X-Api-Request-Id：头 20000000 → body result.text → Transcript；20000001/20000002 → Pending；其余 → ProviderError(头 X-Api-Message)。
- **端到端测试（多连接档案的验证件）**：真 DB —— 建 provider（platform "ark"，default 连接指 wiremock A）+ `provider_connections` upsert role="voice"（base_url=wiremock B，auth_scheme="volc_voice"，credentials 加密存储）+ 模型行 tasks=["speech_recognition"]（protocol 留空→路由表→volc.asr_file@voice）→ `invoke.invoke(SpeechRecognition)` → 断言请求打到 wiremock B 且带三个 X-Api-* 头 → submit/query 两段 → Transcript。**该测试证明：同一供应商不同模态走不同域名+不同凭证。** 另：删除 voice 连接后同调用 → MissingConnection。

- [ ] **Step 1-4:** 红 → 实现 → 绿 → Commit `feat(model-invoke): Volcengine speech ASR adapter over per-role connection profiles`

---

### Task 11: 收尾 —— 全量验证 + 文档 + 交接

- [ ] `cargo test -p nomifun-model-invoke -p nomifun-creation -p nomifun-shell -p nomifun-ai-agent -p nomifun-system -p nomifun-db` 全绿；`cargo test --workspace --exclude nomifun-desktop --no-fail-fast` 与 P0 基线对照（36 个既有失败之外零新增）；`cargo fmt --check`。
- [ ] 更新 spec（P1 状态 + 偏差记录：chat 路径表化推迟 P2、bedrock 不经 invoke、STT legacy 内嵌 key 模式处置结果、多 key 轮换推迟）；新 handoff `docs/handoffs/2026-07-29-model-invoke-p1.md`（交付物、新 crate 结构、适配器矩阵、被删死码、P2 入口：前端统一选择器 + resolve API 接入 + 管理页连接档案 UI + 克隆切服务端 + TS 契约生成）。
- [ ] Commit `docs: record P1 model invoke layer outcome`

## Self-Review 结论

- 覆盖：spec §6 P1 五项 → 新 crate ✓(T1-5)；registry+InvokeError+鉴权基座、删名字路由 ✓(T1/T6/T7)；探针同管线 ✓(T5/T7)；ark×2+volc voice ✓(T9/T10)；TTS+/api/tts ✓(T8)。有意偏差（T11 文档化）：chat 路径特判表化推迟 P2（风险控制，chat 是非目标）；多 key 轮换保持"取第一个"（现状一致）；bedrock 不经 invoke。
- 执行顺序：1→2→3→4→5→**8**→6→7→9→10→11（T8 先于 T6，因 creation 的 Tts 测试依赖 audio_speech 适配器）。
- 类型一致性：ProtocolAdapter/ResolvedCall/TaskRequest/JobHandle 在 T1 定义，T3/4/8/9/10 实现方消费，T2/5 编排方消费，T6/7 产品方消费——签名以 T1 块为准。
- 执行时待核实：前端是否仍发 legacy STT 形态（T7 内二选一已写明）；ark 平台字符串实际值（`MODEL_PLATFORMS` 里查 "ark"/"volcengine"，routes_table 两个都收）。
