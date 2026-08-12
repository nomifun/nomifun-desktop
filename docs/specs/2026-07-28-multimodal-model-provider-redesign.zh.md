# 多供应商 · 多模态模型管理与协议适配重构设计（v2）

> [!CAUTION]
> **已被 2026-08-11 现行规范替代。** 本文仅保留为历史调研、阶段实施与决策记录，不再是模型管理的开发约束。当前的单一能力源、九模态、协议 manifest、自由模型 ID 与每模态 transport 配置以[《模型供应商 × 模态官方接口核验矩阵（2026-08-11）》](./2026-08-11-provider-modality-official-matrix.zh.md)为准；下文的旧表结构、旧选择/解析接口和过渡方案不得用于新代码。

- 日期：2026-07-28（v2 修订）
- 状态：设计定稿；P0 已实施（分支 dev/model-catalog-p0-20260728，见 §6 P0 实施偏差记录）；P1 已实施（分支 dev/model-catalog-p1-20260729，见 §6 P1 实施偏差记录）；P2 已实施（分支 dev/model-catalog-p2-20260729，见 §6 P2 实施偏差记录）；P3 已实施（分支 dev/model-catalog-p3-20260730，见 §6 P3 实施偏差记录）
- 范围：供应商/模型配置与管理、模型能力打标、各模态调用链、外部协议适配层、前端模型选择交互
- 调研方式：四路并行代码审查（配置数据模型 / 对话链路协议抽象 / 前端管理与选择 UI / 多模态调用链路）+ 10 家供应商 × 6 模态的真实协议差异调研（附录 C）+ 适配层抽取的 crate 依赖摸底。关键结论均带 `file:line` 或来源 URL。

**v2 修订记录**（相对 v1 的两处结构性修正）：

1. **放弃"OpenAI 约定为中心 + 逃生舱"的协议假设**。协议差异调研证实：非 chat 模态在很多供应商处是**整套独立体系**（独立域名、独立凭证、独立同步/异步语义、非 JSON 传输），不是"路径不同"。因此：连接配置升级为**供应商 × N 连接档案（per-task 域名+凭证）**；适配层以**供应商原生协议族**为一等公民，OpenAI 约定只是其中一族；传输基座为 HTTP-binary/SSE/WS 留位。
2. **纠正依赖方向**。统一调用层是**底层基础能力**（模型管理入口的运行时门面），新建独立 crate `nomifun-model-invoke`；会话功能（文生图工具、图像理解、语音识别、TTS）直接依赖它；创意工坊 `nomifun-creation` 是**独立产品体系**，同样只是它的一个消费者。协议适配器从 creation 中抽出、下沉到 invoke 层。v1 中"由 nomifun-creation 演进承担"的表述作废。

---

## 0. 一句话结论

现状不是"没有多模态设计"，而是**一次已经开工但只完成了三分之一的统一化改造**：权威的 per-model 能力档案（`ModelTask`/`ModelProfile`）与统一端点解析器（`resolve_dispatch_target`）已经落地，但只有健康探针真正消费它们；真实调用链仍依赖模型名启发式、平台字符串特判和三套互相矛盾的能力词表；且整个配置模型只允许一个供应商一条 base_url + 一把 key——这在"非 chat 模态经常是独立域名独立凭证"的现实（附录 C）面前是结构性缺口。本设计：

1. **数据层收敛 + 连接档案化**：一张权威模型实体表（`provider_models`）+ 一张连接档案表（`provider_connections`，供应商可挂 N 套"域名+鉴权+凭证"，模型按任务绑定连接）；
2. **底层统一调用层**：新建基础 crate `nomifun-model-invoke`（typed 任务请求 → `Done|Pending` 任务句柄，统一错误分类，探针与真实调用同一条解析管线）；会话与创意工坊都只是它的消费者；
3. **供应商原生协议适配层**：`ProtocolAdapter` 注册表按 `(协议族, 任务)` 显式路由；异步任务句柄归一；鉴权方案声明化（≥6 种）；传输通道抽象（HTTP-JSON / HTTP-binary / SSE / WS 留位）；
4. **交互闭环**：所有模型选择器统一走后端 `resolve` API 按任务过滤；打标可确认/纠正系统推断；添加供应商向导由平台注册表驱动（含多连接档案引导）。

---

## 1. 背景与目标

### 1.1 问题背景

用户视角的故障模式：在"供应商模型"管理里添加了图像/语音等非对话模型并打了标签，但实际使用大概率失败。根因分两层：

- **配置层**：管理入口按对话模型设计（一个供应商 = 一条 base_url + 一把 key + 一串模型名），无法表达"火山的语音在另一个域名、另一套凭证"这类现实；打的标签大部分链路不消费；
- **调用层**：没有完整的协议适配层。多处硬编码 OpenAI 协议或按名字猜协议；探针与真实调用走不同代码路径，"检测通过"不代表"能用"。

### 1.2 目标

- 用户配置任何模态的模型后，**能被正确的功能入口选到、以正确的协议+凭证调用、失败时给出可行动的错误**；
- 供应商各模态**独立对接**成为常态而非特例：新增 OpenAI 兼容供应商零代码；新增私有协议模态 = 写一个适配器文件 + 注册表加一行；同一供应商的不同模态可以走完全不同的域名/凭证/协议；
- 协议漂移的修改**局限在单个适配器**；用户侧有 per-model `endpoint`/`request_shape`/params 逃生舱可**不发版自救**；
- 探针与真实调用共享同一解析管线，检测结果可信；
- **依赖方向正确**：模型调用能力是底层基础设施，会话、创意工坊、伙伴、知识库都是平级消费者，任何产品域不得成为另一个产品域的模型能力通道。

### 1.3 非目标

- **不重写 chat 链路**。`nomi-providers` 的 `LlmProvider`（双协议+双承载、流式状态机、staged→原子提交语义）是全仓最健壮的一层（`crates/agent/nomi-providers/src/lib.rs:23-27`），本设计只在目录/选择器/错误分类层面与它对齐。
- 实时语音（双向 WS 会话）不在本期实施，但传输通道抽象为其留位（§4.4、附录 C 要点 5）。
- api_key 明文出 wire、加密密钥同目录等安全问题单列 §9，建议独立立项。

---

## 2. 现状审查结论（摘要）

> 完整证据与 file:line 见 v1 审查（本节保留结论）；新增 §2.5 配置模型缺口。

### 2.1 分层现状

- **chat**：健壮（`LlmProvider` + `ProviderCompat` 15 开关）；adapter 按 `platform` 字符串硬编码映射，其余一律 openai（`nomifun-ai-agent/src/factory/nomi.rs:1089-1111`）。
- **媒体生成**：`nomifun-creation` 的 `MediaProvider` trait（submit/poll）+ 5 适配器；路由按"模型名含 gemini"分派（`adapters/mod.rs:79-81`）；ark/modelscope 空 stub。
- **STT**：`nomifun-shell` 独立双实现，协议靠名字启发式选择（名字含 nova-2 → Deepgram）。
- **统一 taxonomy**：`ModelTask`(8 任务)/`ModelProfile`（`model_profiles` 表）+ `resolve_dispatch_target`；消费者只有健康探针、openai_images、stt_openai 三处。
- **TTS/Embedding/Rerank**：有类型无实现；**三套遗留图像栈**并存、两套死码（含 `nomifun-mcp` 的 `build_builtin_image_gen_server`，经依赖摸底确认全仓无生产调用方）。

### 2.2 三套能力词表并存

`ModelType`（provider 级，后端零业务消费）/ `ModelTask`+`ModelTrait`（权威，per-model）/ `MediaCapability`（creation 专用）——外加 Rust/TS 双份名称启发式需人工同步。

### 2.3 per-model 元数据碎片化

providers 行上 6 个以模型名为键的平行 JSON map（整 map 替换语义并发丢数据、克隆丢标签、删除留孤儿 profile、health 由客户端 PUT 写入可伪造）。

### 2.4 关键断裂点（12 项，v1 §2.4 全文保留）

选择器不认标签 / 探针≠运行时 / 名字含 gemini 误路由 / 国产图像视频模型必失败 / TTS 与 v2v 断头 / STT 名字猜协议 / 默认模型不看任务 / 参数硬透传 / platform 字符串过载 / 错误降级字符串 / 契约手工同步 / 绑定快照反范式。

### 2.5 配置模型的结构性缺口（v2 新增）

`providers` 表只有**单一** `base_url`（`001_v3_baseline.sql:160`）与**单一** `api_key_encrypted`（:161），没有任何 per-task/per-modality 连接机制；多 key 只是"一个加密串内逗号分隔"。而真实世界（附录 C）：

- **火山引擎**：chat/图像/视频在 `ark.cn-beijing.volces.com`（Bearer ARK key），TTS/ASR 在 `openspeech.bytedance.com`，凭证是**完全独立**的 appid/token/cluster（v1）或 `X-Api-App-Key`/`X-Api-Access-Key`/`X-Api-Resource-Id` 四头（v3），Ark key 不可用；
- **MiniMax**：国内/国际双平台 key 不互通，TTS 还要 URL 上带 `GroupId`；
- **Deepgram/ElevenLabs**：纯语音厂，与任何 chat 供应商无关，鉴权是 `Token` 前缀/`xi-api-key` 自定义头。

结论：**"供应商 = 一条连接"的模型必须废弃**。唯一的现存先例是 `bedrock_config`（平台限定的类型化 JSON 列，`nomifun-api-types/src/provider.rs:118-127`），本设计将其一般化为连接档案。

---

## 3. 目标架构与依赖关系

### 3.1 分层与依赖方向（v2 核心修正）

```
                    ┌──────────────────────────────────────────────────┐
                    │        前端（统一交互层：管理页 + TaskModelSelect）│
                    └──────────────────────┬───────────────────────────┘
                                           │ REST
        ┌──────────────────────────────────┴──────────────────────────────────┐
        │                        产品功能层（平级消费者）                        │
        │                                                                      │
        │  nomifun-conversation        nomifun-creation       nomifun-shell    │
        │  （普通会话：文生图工具、      （创意工坊：独立产品     （语音识别入口）  │
        │   图像理解*、ASR、TTS）        体系，任务队列/画布/     nomifun-mcp     │
        │  nomifun-companion            资产管线归它自己）      nomifun-knowledge│
        │  nomifun-idmm / gateway …                            （未来 embedding）│
        └───────┬──────────────────────────────┬──────────────────┬────────────┘
                │            全部单向依赖 ↓（任何产品域不经过另一个产品域拿模型能力）
        ┌───────┴──────────────────────────────┴──────────────────┴────────────┐
        │            nomifun-model-invoke（新 · 底层基础能力 crate）             │
        │   = 模型管理入口的运行时门面：                                          │
        │   ① 目录读取（providers + provider_connections + provider_models）     │
        │   ② 统一任务契约 TaskRequest → TaskOutcome{Done|Pending}               │
        │   ③ ProtocolAdapter 注册表（按 (协议族, 任务) 路由）                    │
        │   ④ 传输基座（鉴权方案/重试/key 轮换/超时/HTTP-JSON·binary·SSE）        │
        │   ⑤ 统一错误分类 InvokeError / 探针 probe()（与 invoke 同管线）         │
        │   依赖：nomifun-common / nomifun-api-types / nomifun-db / nomifun-net  │
        │   —— 不依赖任何产品功能 crate                                          │
        └───────────────────────────────┬───────────────────────────────────────┘
                                        │
        （chat 族保持现状：nomi-providers 流式引擎，经 nomifun-ai-agent 接缝；
          与 invoke 层共享目录/错误分类/选择器，不共享执行路径）
                                        │
                                        ▼
                            外部供应商 API（多域名/多凭证/多协议族）
```

\* 图像理解 = chat 的 `vision_input` trait，执行仍走 chat 族；列在会话依赖链里是完整性说明。

**依赖规则（红线）**：

1. `nomifun-model-invoke` 只依赖数据层与共享层（`nomifun-common`/`nomifun-api-types`/`nomifun-db`/`nomifun-net`），**不依赖**任何产品功能 crate；
2. 会话（`nomifun-conversation`、内置工具）、创意工坊（`nomifun-creation`）、STT（`nomifun-shell`）、伙伴/知识库/gateway 各自**直接**依赖 invoke，互相之间不为模型能力建立依赖；
3. `nomifun-creation` 保留的是**产品体系**：任务队列、画布、资产 source/sink、产物落盘与回滚（`service.rs`/`artifact.rs` 的业务侧、`workshop_bridge`）；**协议适配器与 provider 解析从它抽出**下沉到 invoke；
4. 会话内文生图不再走"前端拼 env 注入 MCP"的死码路径，改为后端内置工具直调 `ModelInvokeService`（先例：gateway 画布工具直调 CreationService，`nomifun-gateway/src/caps_workshop.rs:17`、`deps.rs:66`）。

**抽取可行性（依赖摸底证据）**：creation 的 `provider.rs` + `adapters/*` 只被 `service.rs` 单向引用，自身不引用任务队列/资产管线（`adapters/mod.rs:24-25` 只依赖 provider/types；service 反向引用清单在 `service.rs:32-41,895,1027`）——可整体搬迁。共享类型结（`MediaCapability`/`CreationError`，`types.rs:9-57,98-139`）的处理：适配层统一改用 `ModelTask` + `InvokeError`，creation 侧保留自己的请求词表并做 `From` 映射（`model_task.rs:1-3` 本就声明 ModelTask 取代 MediaCapability）。workspace 登记走 glob members，零配置（根 `Cargo.toml:3`）。

### 3.2 数据模型：供应商 → N 连接档案 → 模型按任务绑定连接

```
providers            供应商实体（platform、名称、排序、启用；不再承载连接细节）
   │ 1:N
provider_connections 连接档案（域名 + 鉴权方案 + 凭证 JSON；"默认连接"兼容现状）
   │ 1:N（模型/任务绑定）
provider_models      模型实体（tasks/traits/协议覆盖/params/启用/健康/上下文/描述…）
```

DDL 草案（遵守 v3 契约：无物理外键、逻辑关联、UUIDv7 业务 id）：

```sql
CREATE TABLE provider_connections (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_id TEXT NOT NULL UNIQUE,      -- UUIDv7
    provider_id TEXT NOT NULL,               -- 逻辑关联 providers
    role TEXT NOT NULL DEFAULT 'default',    -- 平台注册表定义的连接角色：default/ark/voice/…
    label TEXT,                              -- "方舟网关" / "火山语音技术"
    base_url TEXT NOT NULL,
    auth_scheme TEXT NOT NULL DEFAULT 'bearer',
    credentials_encrypted TEXT NOT NULL,     -- 类型化 JSON（按 auth_scheme 校验）：
                                             --   bearer: {"api_keys":["sk-…"]}
                                             --   volc_voice: {"app_id":…,"access_token":…,"cluster":…}
                                             --   aws_sigv4: {…}（bedrock_config 一般化）
    is_full_url INTEGER NOT NULL DEFAULT 0,
    extra TEXT NOT NULL DEFAULT '{}',        -- 连接级杂项（如 MiniMax GroupId、区域）
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    UNIQUE(provider_id, role)
);

CREATE TABLE provider_models (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL,
    model TEXT NOT NULL,
    connection_role TEXT,                    -- NULL = 按平台注册表按任务解析；显式值 = 用户覆盖
    enabled INTEGER NOT NULL DEFAULT 1,
    sort_order INTEGER NOT NULL DEFAULT 0,
    tasks TEXT NOT NULL DEFAULT '[]',        -- Vec<ModelTask>
    traits TEXT NOT NULL DEFAULT '[]',
    protocol TEXT,                           -- 协议族覆盖（NULL = 平台注册表解析）
    params TEXT NOT NULL DEFAULT '{}',       -- endpoint/request_shape/size/voice/… 逃生舱
    context_limit INTEGER,
    description TEXT,
    source TEXT NOT NULL DEFAULT 'inferred',
    health TEXT, health_checked_at INTEGER,  -- 只由服务端探针写
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    UNIQUE(provider_id, model)
);
```

迁移与兼容：

- 现状的 `base_url + api_key_encrypted + is_full_url + bedrock_config` 迁成每个 provider 的 `role='default'` 连接档案（bedrock_config → `auth_scheme='aws_sigv4'` 的凭证 JSON）；`model_profiles` + 6 个平行 map 合并入 `provider_models`；
- `ProviderResponse` 兼容投影期继续拼出旧形状（由默认连接反推），新增 `connections[]` 与 `models_detail[]` 字段；一个版本后删旧字段；
- 克隆丢 profile、删除留孤儿、health 客户端写路径三个数据 bug 随迁移一并修复。

**协议与连接的解析链**（目录层唯一算法）：

```
task ∈ model.tasks？ —否→ InvokeError::UnsupportedTask
protocol   = model.protocol ?? PlatformManifest[platform].tasks[task].protocol ?? "openai"
connection = model.connection_role
             ?? PlatformManifest[platform].tasks[task].connection
             ?? "default"
             → provider_connections 行（缺失 → typed 错误："该任务需要配置 XX 连接"）
endpoint   = model.params.endpoint 覆盖 ?? 协议族默认路径规则(connection.base_url, task)
```

### 3.3 平台注册表（声明式，多连接角色）

一处定义（内置 Rust 常量表/内嵌 TOML），承接现在散落 5+ 处的平台特判，并声明**连接角色**：

```toml
[platform."volcengine"]
display_name = "火山引擎"
model_fetch = "openai_models"                # 在 default/ark 连接上拉模型列表
[[platform."volcengine".connections]]
role = "ark"    # 兼作 default
label = "方舟网关"
auth = "bearer"
default_base_url = "https://ark.cn-beijing.volces.com"
[[platform."volcengine".connections]]
role = "voice"
label = "语音技术（独立开通）"
auth = "volc_voice"                          # X-Api-* 四头 / appid+token+cluster
default_base_url = "https://openspeech.bytedance.com"
optional = true
[platform."volcengine".tasks]
chat               = { protocol = "openai",          connection = "ark", base_path = "/api/v3" }
image_generation   = { protocol = "ark.images",      connection = "ark" }
video_generation   = { protocol = "ark.video_jobs",  connection = "ark" }
speech_synthesis   = { protocol = "volc.tts_v3",     connection = "voice" }
speech_recognition = { protocol = "volc.asr_file",   connection = "voice" }

[platform."gemini"]
auth = "header_key:x-goog-api-key"
[platform."gemini".tasks]
chat             = { protocol = "openai", base_path = "/v1beta/openai" }   # 现状规则入表
image_generation = { protocol = "gemini.generate_content" }
speech_synthesis = { protocol = "gemini.generate_content_audio" }
# 注意：Gemini 的 OpenAI 兼容层无 /audio/*，图像仅认 5 参数（附录 C）——兼容层可用面按任务声明
```

UI 含义：添加"火山引擎"供应商时向导给出两个连接卡片（方舟必填、语音可选）；未配语音连接时，打了 TTS/ASR 标签的模型在选择器里显示"需配置语音连接"禁用态，而不是选中后失败。

`nomifun-free-model` 托管平台是注册表中 `managed = true` 的一项，行为不变。

### 3.4 统一调用层 `nomifun-model-invoke`

对产品功能暴露的唯一入口（typed core + `extra` JSON 双层参数）：

```rust
pub struct ModelRef { pub provider_id: ProviderId, pub model: String }

pub enum TaskRequest {
    ImageGeneration(ImageGenRequest),    // prompt/count/size?/inputs[]/extra
    ImageEdit(ImageEditRequest),         // inputs+mask?/mode(inpaint)/extra
    VideoGeneration(VideoGenRequest),    // prompt/inputs?/seconds?/resolution?/extra
    SpeechSynthesis(TtsRequest),         // text/voice?/format?/extra
    SpeechRecognition(AsrRequest),       // audio/language?/extra
    Embedding(EmbedRequest),             // inputs[]/extra
    Rerank(RerankRequest),               // query/documents[]/extra
}

pub enum TaskOutcome { Done(TaskResult), Pending(JobHandle) }

impl ModelInvokeService {
    pub async fn invoke(&self, m: &ModelRef, req: TaskRequest) -> Result<TaskOutcome, InvokeError>;
    pub async fn poll(&self, job: &JobHandle) -> Result<TaskOutcome, InvokeError>;
    /// 探针 = 最小请求体走同一条 invoke 管线
    pub async fn probe(&self, m: &ModelRef, task: ModelTask) -> Result<ProbeReport, InvokeError>;
    /// 产物物化：受 cap 的下载 + MIME/magic 校验（供应商 URL 普遍 24h~2 天过期，
    /// 调用方必须立即转存——会话存工作区，工坊存资产库）
    pub async fn materialize(&self, asset: &ProducedAsset) -> Result<LoadedAsset, InvokeError>;
}
```

**解析管线（invoke 与 probe 共用同一条）**：

```
1. 目录读取：providers 行 + provider_models 行 + 解析出的 provider_connections 行（解密凭证）
2. 守门：provider.enabled ∧ model.enabled ∧ task ∈ model.tasks（违反 → typed 错误）
3. 协议/连接解析链（§3.2）
4. adapter = AdapterRegistry[(protocol, task)]（缺失 → NoAdapter{protocol, task}）
5. ResolvedCall { connection(base_url+auth material), model, task, request, model_params }
6. 传输基座执行：鉴权方案应用、key 轮换（401/403/429）、重试（连接错/5xx/Retry-After）、
   分级超时、日志脱敏
```

**统一错误分类**（chat 与 media 语义对齐，跨边界不再降级为字符串）：

```rust
pub enum InvokeErrorKind {
    Auth, RateLimited { retry_after_ms: Option<u64> }, QuotaExhausted,
    UnsupportedTask, NoAdapter, MissingConnection { role: String },
    InvalidParams { hint: Option<String> }, ContentPolicy,
    ProviderError { status: u16 }, Network, Timeout, ParseError, CapabilityRejected,
}
```

`CreationError`/`SttError` 收敛映射；`nomi-providers::ProviderError` 提供 `From` 转换，使 IDMM/failover/vision 否定缓存等"按错误语义自愈"回路可泛化。

### 3.5 协议适配层：供应商原生协议族为一等公民

```rust
#[async_trait]
pub trait ProtocolAdapter: Send + Sync {
    fn id(&self) -> &'static str;                       // "ark.video_jobs"
    fn supports(&self, task: ModelTask) -> bool;
    async fn submit(&self, call: &ResolvedCall) -> Result<TaskOutcome, InvokeError>;
    async fn poll(&self, call: &ResolvedCall, job: &JobHandle) -> Result<TaskOutcome, InvokeError>
        { Err(InvokeError::not_pollable()) }
}
```

**注册表查找键只允许 `(protocol, task)`**——任何按模型名子串/平台名 if 的路由一律清除；名称启发式只允许存在于目录层播种函数。

调研落到设计上的四个硬机制（证据见附录 C 设计要点）：

1. **鉴权方案声明化**（≥6 种，由连接档案声明、传输基座实现，适配器不再各自拼 header）：

```rust
pub enum AuthScheme {
    Bearer,                                   // 多数
    TokenHeader,                              // Deepgram: Authorization: Token …
    HeaderKey { header: &'static str },       // x-goog-api-key / xi-api-key
    MultiHeader,                              // 火山 v3：X-Api-App-Key/Access-Key/Resource-Id(+Request-Id)
    BodyEmbedded,                             // 火山 TTS v1：凭证嵌请求体 app 块（含 "Bearer;token" 畸形头）
    QueryKey { param: &'static str },         // query 传 key / MiniMax GroupId 类连接参数
    AwsSigV4,                                 // bedrock（现 bedrock_config 一般化）
    OAuth2ServiceAccount,                     // vertex
}
```

2. **异步任务句柄归一**。没有任何两家 submit/poll 一致（id 字段名 task_id/id/requestId/operation-name；id 位置 URL 路径/query/POST body；轮询端点专属 vs 平台统一；状态词表 5+ 套；火山 ASR 状态在响应头；DashScope 靠 `X-DashScope-Async` 头区分同异步；火山/DashScope-WS 由客户端发号）。内部统一为：

```rust
pub struct JobHandle {
    pub adapter_id: String,
    pub remote_id: String,
    pub id_origin: IdOrigin,          // ServerIssued | ClientGenerated
    pub poll_state: serde_json::Value // adapter 私有（轮询端点模板/复用 header/file_id 等）
}
pub enum JobStatus { Pending, Running, Succeeded, Failed, Canceled }  // 词表归一
```

词表映射、轮询方式、结果取回跳数（0~2 跳：poll 即 URL / 另调 content 端点 / file_id 换 download_url）全部封装在各 adapter 的 `poll` 里；上层只见归一状态与 `ProducedAsset`。

3. **传输通道抽象**（基座提供，adapter 声明使用哪种）：`HttpJson` / `HttpBinary`（上行 raw body：Deepgram；下行裸二进制/分块流：OpenAI-TTS、ElevenLabs）/ `Sse`（MiniMax hex 分块、火山 v3 JSON-lines）/ `WebSocket`（本期只留位，P4 实施）。音频编码（base64/hex/raw）是通道之上的 codec 声明——MiniMax 的 hex 教训入表。

4. **参数规整器 + 白名单**。"OpenAI 兼容"在非 chat 模态普遍是残缺子集（Gemini 兼容层图像仅认 5 参数且**静默忽略**其余；gpt-image-1 与 dall-e 同端点但词表冲突）。每个 adapter 声明参数白名单与词表映射（quality: high→hd 等），不透传；裁剪/映射行为通过 `warnings[]` 回报给调用方与 UI。

**首批适配器矩阵**（M=迁移现有代码，N=新写）：

| 适配器 id | 任务 | 来源 |
|---|---|---|
| `openai.images` / `openai.videos` / `openai.audio_speech` / `openai.audio_transcriptions` / `openai.embeddings` | 图像/视频/TTS/ASR/嵌入 | M：creation 与 shell 现有实现迁入（openai_video 接入统一解析器，修 `params.endpoint` 对视频无效问题） |
| `gemini.generate_content`（图/文/TTS 变体） | 图像/TTS | M+N：迁 gemini_image/gemini_text；补 count>1 循环与 TTS `responseModalities:["AUDIO"]` |
| `deepgram.listen` | ASR | M：迁 stt_deepgram（Token 鉴权与 raw-body 进基座） |
| `ark.images` / `ark.video_jobs` | 火山图像/视频 | N：填现有空 stub（P1 扩展性验证件） |
| `volc.tts_v3` / `volc.asr_file` | 火山语音 | N：**多连接档案的验证件**（独立域名+四头鉴权+状态在响应头） |
| `dashscope.aigc_image` / `dashscope.asr_file` / `dashscope.embedding` / `dashscope.rerank` | 通义系 | N：P2（强制异步头、统一 /tasks/{id} 轮询、input/parameters 包裹结构） |
| `minimax.t2a` / `minimax.video_jobs` | MiniMax | N：P3（hex codec、GroupId、三段式取回） |
| `zhipu.images` / `zhipu.video_jobs`、`elevenlabs.tts`、`stepfun.*` | — | N：按需，成本=单文件 |

### 3.6 前端与交互重构

1. **统一选择器**：`TaskModelSelect` + `useModelsForTask(task, traits?)`（数据源 = `modelProfile.resolve`，含健康与禁用原因），替换 ≥10 处各自实现；对话/定时任务/协作/伙伴/IDMM/故障转移 → `chat`（带附件加 `vision_input`）；绘图/视频/ASR/TTS 各按任务。禁用态可解释（"需配置语音连接"/"探针失败：Auth"）。
2. **管理页升级**：供应商卡片 = 基本信息 + **连接档案区**（按平台注册表渲染角色卡片，凭证表单按 auth_scheme 生成）+ 模型实体行（任务徽章/协议标签/连接角色/params 高级抽屉/健康只读）；Inferred 档案可见可一键确认；健康检查按钮传 task。
3. **绑定引用化**：一律 `{provider_id, model}`，`TProviderWithModel` 全量快照淘汰。
4. **契约生成**：provider/connection/model 域纳入 ts-rs 管线；删除 TS 侧启发式双胞胎与 allowlist。
5. **清理遗留栈**：删除 imageGenCore（form B）、`NOMIFUN_IMG_*` env 注入（`build_builtin_image_gen_server` 已确认死码）与 `tools.imageGenerationModel` 键；会话内文生图改为后端内置工具直调 invoke；agent 侧产物契约（`tool_proxy.rs:148-152`、`output/mod.rs:635-667`）对接新工具名。

---

## 4. API 变更摘要

| 端点 | 变化 |
|---|---|
| `/api/providers*` | 保留；响应加 `connections[]`、`models_detail[]`；旧字段兼容投影期 |
| `GET/POST/PATCH/DELETE /api/providers/{id}/connections/{role}` | **新增**：连接档案 CRUD（凭证按 auth_scheme 校验，只写不回读明文） |
| `GET/POST/PATCH/DELETE /api/providers/{id}/models/{model}` | **新增**：模型实体行级 CRUD（替代整 map 替换） |
| `POST /api/model-profiles/resolve` | 升级为选择器唯一数据源（含健康/禁用原因/缺连接提示） |
| `POST /api/agents/provider-health-check` | 内部改走 `ModelInvokeService::probe`；结果回写 `provider_models.health` |
| `POST /api/creation/tasks` | 入参不变；`CreationService` 改为消费 invoke 层（保留自身任务队列/资产管线） |
| `POST /api/stt` | 内部改走 invoke（协议来自注册表而非名字猜测） |
| `POST /api/tts` | **新增**：SpeechSynthesis 产品入口（会话/伙伴共用） |

---

## 5. 关键设计决策与理由

| 决策 | 备选 | 理由 |
|---|---|---|
| **供应商 × N 连接档案，模型按任务绑定连接** | 单 base_url + per-model endpoint 覆盖（v1 方案） | 火山双域双凭证、MiniMax 双平台、纯语音厂的存在使"逃生舱补丁"不成立；凭证形状本身随模态变（appid/token/cluster ≠ api key），必须是结构化连接实体。bedrock_config 是"类型化凭证 JSON"的现成先例 |
| **独立底层 crate `nomifun-model-invoke`，适配器从 creation 抽出** | 由 creation 演进承担（v1 表述） | 依赖方向：模型调用是基础能力，会话/工坊是平级消费者；工坊是独立产品体系不应成为会话的能力通道。摸底证实 adapters+provider.rs 单向无环可整体搬迁 |
| **协议族为一等公民，OpenAI 约定只是一族** | OpenAI 约定 + 差异打补丁 | 附录 C：兼容层在非 chat 模态普遍残缺且"静默忽略"参数；异步/传输/编码差异不是路径级而是体系级 |
| chat 链路不动 | 塞进统一 trait | 流式状态机与工具调用强绑定，泛化无收益有风险 |
| submit/poll + JobHandle 归一 | 全同步/全异步 | creation 已验证范式；调研显示异步细节五花八门，恰恰需要句柄抽象吸收 |
| typed core + `extra` + 参数白名单规整 | 全 typed / 全 opaque 透传 | 全 typed 追不上演化；透传 = 必炸（gpt-image-1 拒收 response_format 实证） |
| protocol/连接角色为开放字符串 + 注册表 | 封闭 enum | 扩展不动 wire 契约 |
| per-model `endpoint`/`request_shape`/params 逃生舱保留并 UI 化 | 只靠发版 | 协议漂移时用户即时自救 |

---

## 6. 迁移路径（每阶段可独立发版）

### P0 数据收敛（无行为变化）
1. 建 `provider_connections` + `provider_models` 表；迁移（单连接→default 档案、bedrock_config→sigv4 凭证、model_profiles+6 map→模型行）；
2. 连接/模型行级 API 上线；`ProviderResponse` 兼容投影；
3. 修三个数据 bug（克隆丢标签/孤儿 profile/health 客户端写）；provider 变更时同步播种 profile。

验收：旧前端不感知；数据 100% 迁入；克隆后标签保留。

#### P0 实施偏差记录（2026-07-28，分支 dev/model-catalog-p0-20260728，迁移 014/015）

与上文设计的五处有意偏差，均计划在 P2 收缩期消化：

1. **default 连接不迁入 `provider_connections`**：providers 行自身继续充当 default 连接（base_url/api_key 等仍在 providers 表），`provider_connections` 只存附加角色（如 volc 型多域名/多凭证档案）。增量式演进，避免 P0 触碰所有现有读写方；P2 再决定是否完全档案化。
2. **旧 6 个 map 列物理保留并被双写**：读路径已切到 provider_models 行投影（`ProviderResponse` 不再读 map 列），但 create/update 仍同步写旧列，防止仓内直接读写方漂移；P2 收缩期物理删除。
3. **`DELETE /api/model-profiles` 语义放宽**：model_profiles 表已删（迁移 015），该端点现在删除的是 provider_models 目录行（原先仅删 profile 覆盖层）。无存量调用方受影响，wire 形状不变。
4. **health 行字段成为服务端权威**：探针结果由服务端直接持久化到 provider_models.health；客户端 PUT model_health map 的旧写路径仍接受（wire 兼容），但行数据为权威，P2 切换 UI 后关闭旧写路径。
5. **克隆修复仅服务端交付**：`POST /api/providers/{id}/clone` 已上线，被调用时完整保留模型行与连接档案，但设置页 UI 仍使用遗留客户端克隆（`ui/src/renderer/utils/model/providerClone.ts`，将模型重建为无 profile 行）——"克隆后标签保留"的验收在 P2 前端切换到该端点之前仅服务端满足，克隆丢标签的用户可见症状届时才消除。

### P1 invoke crate 成型 + 适配器迁移（修复"必失败"）
1. 新建 `crates/backend/nomifun-model-invoke`（glob member 自动登记）：搬迁 `provider.rs`+`adapters/*`+共享 HTTP 助手与格式校验（creation 侧改 5 组 use，`service.rs:32-41`）；平台注册表替换 `map_nomi_provider`/`resolve_nomi_url_and_compat` 散落特判（行为快照测试保护）；
2. `AdapterRegistry` + `InvokeError` + 鉴权方案基座；**删除 `is_gemini` 名字路由与 STT 名字猜协议**；creation 与 shell 改为消费 invoke；
3. 探针改走 `probe()` 同管线（Deepgram 鉴权/视频 multipart 不一致随之消失）；
4. `ark.images`/`ark.video_jobs` 落地（填 stub）；`volc.tts_v3` 或 `volc.asr_file` 至少一个落地（**多连接档案端到端验证件**）；
5. TTS 适配器（openai.audio_speech）+ `/api/tts` 入口。

验收：StepFun 图像、火山图像/视频/语音（双连接）、Deepgram ASR、OpenRouter 上 gemini 命名模型全部真实调用成功；探针结果与真实调用一致。

#### P1 实施偏差记录（2026-07-29，分支 dev/model-catalog-p1-20260729）

与上文设计的八处有意偏差（来源：计划 Self-Review、SDD ledger 与各任务报告），除注明者外计划在 P2 及后续阶段消化：

1. **chat 路径特判表化推迟 P2**：`map_nomi_provider`/`resolve_nomi_url_and_compat` 的散落平台特判未动，`nomi-providers` 不变。P1 的平台路由表（`routes_table.rs`）只服务非 chat 任务；chat 是本设计的非目标（§1.3），表化统一作为风险控制推迟到 P2。
2. **bedrock 不经 invoke 层**：`platform=="bedrock"` 且走 default 连接时，解析器返回 typed `Config` 错误（"bedrock is not supported by the invoke layer yet"），而非尝试 SigV4。AwsSigV4 鉴权方案未实现。
3. **多 key 轮换保持"取第一个"**：default 连接的凭证由 providers 行加密串按逗号/换行分割后取第一个非空 key（`AuthMaterial::primary_secret` 取 `api_keys[0]`），与现状一致；基座级 key 轮换与 per-key 健康推迟（§8-2）。
4. **STT legacy 内嵌 key 模式退役**（wire 行为变化）：`/api/stt` 偏好中无 provider_id、且 openai/deepgram 内嵌 config 携带**非空 api_key** 的旧形态，现在返回 500（STT_UNKNOWN，消息引导用户在设置中重选供应商）；无 provider_id 且内嵌块为空壳（api_key 为空——前端历史上会为未配置的供应商持久化空 key 壳）的形态**不触发退役守卫**，照旧回落 STT_OPENAI/DEEPGRAM_NOT_CONFIGURED 的 400 族（边界钉测于 nomifun-shell/tests/stt_integration.rs 与 app e2e st5/st6，commit 4b13ece7）。执行前 grep 确认前端 UI 早已只写 provider_id 模式，故按计划 T7 的二选一裁决直接退役；存量旧偏好的一次性迁移列为后续改进。
5. **V2v 错误码改为 `unsupported_capability`**：创意工坊 V2v capability 原返回 `adapter_unavailable`，现统一为 `unsupported_capability`（与 invoke 层 UnsupportedTask 语义对齐，任务简报裁定）。
6. **gemini 命名模型在非 gemini 平台行上需要行级 protocol 覆盖**：名字嗅探路由（"模型名含 gemini"）已铲除；OpenRouter 等聚合平台上的 gemini 命名模型若需走 gemini 原生协议，逃生舱 = `provider_models.protocol` 行级覆盖（设计内行为，此处记录以对照 P1 验收中"OpenRouter 上 gemini 命名模型"一项的达成方式）。
7. **rerank 任务有路由无适配器**：路由表已声明 `openai.rerank`，但适配器未实现——调用得到诚实的 `NoAdapter` typed 错误（原设计矩阵中 rerank 属 P2 dashscope 批次）。
8. **`volc.tts_v3` 路由已声明、适配器未实现**：路由表 volcengine/ark 平台的 SpeechSynthesis 已指向 `volc.tts_v3`@voice 连接，但适配器未落地（P1 验证件选择了 `volc.asr_file`，满足"至少一个"要求）——同样得到 `NoAdapter`。按需在 P2+ 补齐。

### P2 前端统一（修复"选错模型"）
选择器统一走 resolve（含 task 过滤接入默认模型/IDMM/故障转移）；管理页连接档案区 + 模型实体行 + Inferred 确认；ts-rs 契约；发送链路 vision 守门；dashscope 系适配器。

#### P2 实施偏差记录（2026-07-30，分支 dev/model-catalog-p2-20260729，迁移 016）

与上文设计的有意偏差（来源：计划 Self-Review、SDD ledger 与各任务报告），除注明者外计划在 P3 消化：

1. **`providers.capabilities` 列保留至 P3**：迁移 016 删除了 6 个 legacy per-model map 列，但 `capabilities`（provider 级旧词表 `ModelType`/`ModelCapability`）有意保留——前端唯一实际消费者随 T4/T5 的启发式清场移除后，该列成为纯写入死数据（wire 上仍收发）。本期不删的理由：避免同一发版窗口对 providers 表做第二次 ALTER；列删除与 `ModelType` 旧词表退役一并列为 P3 收缩项。
2. **legacy STT 偏好一次性迁移 = disable + 去凭证，非静默转换**：boot 时（`nomifun-app/src/services.rs::migrate_legacy_speech_preference`）读 `tools.speechToText` 与旧键 `speechToText`，对"无 provider_id 且内嵌 openai/deepgram 块携带非空 api_key"的 P1 已退役形态：置 `enabled: false`、删除内嵌凭证块、其余字段保留，`tracing::info!` 引导用户在设置中重选供应商。**不**尝试把内嵌凭证自动转换成目录 provider（凭证归属/域名语义无法可靠推断）；幂等（二次启动无内嵌凭证即不触碰）；空 key 壳与已有 provider_id 的配置一律不动。
3. **心跳健康持久化仍走 legacy `model_health` map PUT 写路径**：行级 `provider_models.health` 由服务端探针权威写入（P0 已落地，本期 `stamp_model_unhealthy` 也改为行级 `set_health`），但设置页心跳把探针结果回写时仍使用 `updateProvider({model_health})` 整 map 兼容写（fetch-latest-then-merge）。该 map 参数 wire 上继续接受并驱动行同步，行为正确但残留读改写窗口；UI 切行级写 + 关闭 PUT 兼容路径留到 P3。
4. **dashscope 系适配器推迟 P3**：上文 P2 行内的"dashscope 系适配器"未实施——P2 计划裁定按需适配器（dashscope/minimax/volc.tts_v3）统一记 P3 入口；路由表已声明的任务在适配器缺席时得到诚实的 `NoAdapter` typed 错误（P1 偏差 7/8 的延续）。
5. **ts-rs 契约生成未管线化，交付为手写镜像 + serde 钉测**：provider/connection/model 域的 TS 类型（`ui/src/common/types/provider/providerModel.ts`、`providerConnection.ts`）为手工转写，带"keep in sync"指针注释与 wire key 集钉测（round-trip/tri-state/deny_unknown_fields 安全性）；ts-rs 自动生成管线列 P3。另：`mode.updateProvider` 桥接层防御性剥除 `models_detail`（否则整 spread 的读改写调用点会因后端 `deny_unknown_fields` 全部 400）。
6. **行绑定存储的三处语义变化（T2 裁定 Accept）**：① 重新加入 membership 的模型若同调用未带对应 map 参数，从列默认值起步（双写期会继承残留 legacy map 条目）；② 托管免费模型服务对"当前目录缺席模型"的 per-model 禁用开关不再跨重启持久（无行可承载；进程内仍保留）；③ 故障转移候选无行时视为"未禁用/健康未知"（与旧缺 map 条目语义一致，fail-open），供应商存在性仍是硬门。
7. **管理页 Add/Edit 弹窗仍走整 provider map PUT**：模型行内编辑（启用/上下文/描述/协议/高级抽屉/排序/删除）已切行级 `/api/provider-models`，但 AddModelModal/AddPlatformModal/EditModeModal 的 membership 新增仍发整 map 更新（wire 兼容参数，驱动行同步）——按任务范围保留，P3 可随 map 参数退役一并收敛。
8. **chat 平台表化省略 compat 列**：T6 的 `PLATFORM_CHAT_RULES` 常量表（14 行 + 默认行，220 行行为快照字节级锁定）未按草图携带 per-platform compat 覆盖列——现状所有 compat 覆盖均由 URL 规则或 host 门控规则完全决定，该列在全部行上恒为 None（死配置）；后续需要时增列是加法变更。new-api per-model protocol 特例与 `is_full_url` 早退保持逐字。
9. **选择器首次过滤带来的可见收紧（设计内，此处披露）**：IDMM 备用/旁路模型与故障转移候选选择器首次获得 chat 任务过滤——已保存的非 chat 模型值不被清除、继续生效，但不再被重新提供（failover 草稿中显示"(不可用)"）；guid 模型选择原先包含"禁用中的供应商"（潜在 bug），resolve 单源后被排除。

### P3 扩展与清理
minimax/zhipu/elevenlabs 按需适配器；会话内文生图工具直调 invoke 并删三套遗留栈与 `ModelType` 旧词表、TS 启发式；`TProviderWithModel` 引用化完成；Embedding/Rerank 首个消费者（知识库向量化立项即用）。

#### P3 实施偏差记录（2026-07-30，分支 dev/model-catalog-p3-20260730，迁移 017）

与上文设计的有意偏差（来源：P3 计划、SDD ledger 与 task-1/2/345 报告）：

1. **rerank / realtime 不做（YAGNI 决议）**：上文 P3 行内的 rerank 消费者与 §7 的 realtime 模态本期均未实施——全仓无产品消费者（知识库向量化未立项、无实时语音入口）。rerank 路由表声明保留，调用得到诚实的 `NoAdapter`（P1 偏差 7 延续）；realtime 需要 WS 传输通道，属 P4 既定项。首个消费者立项时按 §7 扩展指南补齐。
2. **两处记录在案的 wire 行为变化（服务端 health/capabilities 收口）**：① `Create/UpdateProviderRequest.model_health` 接受但忽略——服务端探针是 `provider_models.health` 的唯一写方（反伪造收口；P2 偏差 3 的关闭动作）。附带 UI 变化：设置页"清除状态"按钮及其 `clearAllHealthData` 整 map PUT 随之删除（服务端忽略后该功能已成 no-op；行级"清除健康状态"能力如需恢复须走行级 API，暂无产品诉求）。② `capabilities` 请求字段接受但忽略、响应恒 `[]`（迁移 017 已物理删列，字段仅为 wire 兼容保留）；FE 的 `IProvider.capabilities` 与 `ModelCapability`/`ModelType` 旧词表类型一并删除。另一处顺带语义变化：探针请求本身传输层失败（IPC 异常）时不再有任何 unhealthy 落库（原 FE 兜底写路径已删、服务端未收到探针）——"探不到 ≠ 不健康"，裁定可接受。
3. **volc.tts_v3 / minimax.t2a 依据差异矩阵实现，标注需真实调用校准**：两适配器（及 dashscope.images 的 size 词表）按附录 C 差异矩阵的 ※ 中置信度条目落地，代码注释标注"接入时需真实调用校准"——volc.tts_v3 的 `req_params` body 形状与 JSON-lines 聚合词表、minimax 的 `voice_setting`/`audio_setting` 缺省省略语义均以 wiremock 快照锁定当前理解，未经真实供应商调用验证。
4. **minimax 需连接档案携带 `extra.group_id`**：`minimax.t2a` 路由走 default 连接，而 resolver 给 default 连接的 `extra` 恒为 `{}`——用户须为模型行设置 `connection_role` 指向携带 `extra.group_id` 的连接档案，否则得到可行动的 Config 错误（提示补 group_id）。设计事实而非缺陷：路由表不含 role，由行级覆盖表达（计划如此裁定）。
5. **多 key 轮换为进程内每调用顺序尝试**：`send_with_rotation` 对 401/403/429 依 `api_keys` 存储顺序换 key 重试（每 key 一次，全败返回最后一个响应交原分类路径→Auth/RateLimited）；仅作用于单 key 型 scheme（Bearer/TokenHeader/HeaderKey/QueryKey），MultiHeader（volc 四头）与 <2 key 单发不变；传输错误不轮换。计划中"可选"的成功 index 进程内 LRU 未做；per-key 健康/冷却持久化仍列 P4 候选（§8-2）。签名偏差：助手收 `Fn() -> Result<RequestBuilder>` 闭包（reqwest builder/multipart Form 不可克隆，须每次重建），非计划草图的 `build(&str)`。顺带行为差：`primary_secret` 对 `api_keys` 首个空串条目不再报错（取第一个非空 key）。
6. **ts-rs 覆盖 provider 新域 15 类型；老 `providerApi.ts` 手写镜像保留**：cargo test 即生成器（复用 `export_binding_if_changed` 模式），15 个类型（ModelTask/ModelTrait/ProfileSource、HealthStatus/ModelHealthStatus/CloneProviderRequest、ProviderModel 四请求响应、ProviderConnection 两型、CatalogModelRef/ResolveModels 请求响应）生成到 `ui/src/common/protocolBindings/`，FE 侧 `providerModel.ts`/`providerConnection.ts`/`storage.ts` 相应类型改纯 re-export。**保留手写**的是 `providerApi.ts` 的 `ProviderResponse`/Create/Update 镜像：这些类型仍承载 legacy map 兼容字段且 FE 刻意不 1:1 镜像 wire（`provider_id`↔`id` 双形状、vestigial `capabilities` 不镜像、`ProviderId` branding）——迁 ts-rs 应与 legacy map 字段退役同批做，避免把过渡形状固化进生成契约。生成形状裁定：双 Option 三态字段生成 `x?: T | null`（absent=keep/null=clear），单 Option 请求字段刻意生成 `x?: T` 无 `| null`（serde 对其 null≡absent，沿用原镜像的更窄意图型）；`serde_json::Value`→`unknown`、i64→`number`（普通 JSON number 非 bigint），均有形状断言测试锁定。

### P4 长期收敛（可选）
WS 传输通道实施（实时语音任务类型）；chat 传输基座下沉 shared crate；平台注册表外置热更新（hub 下发 overlay）。

---

## 7. 扩展指南（重构后的"新增成本"）

- **OpenAI 兼容供应商**：零代码（默认协议族 + 约定路径；路径怪异 → 模型 params.endpoint 覆盖）。
- **多域名/多凭证供应商（火山型）**：平台注册表加连接角色 + 任务映射；若协议族已有适配器则零代码，否则每个新协议族一个适配器文件。
- **纯单模态供应商（ElevenLabs 型）**：注册表一条 + 一个适配器；自动获得 UI/存储/选择器/探针/错误处理。
- **新模态（如 realtime）**：`ModelTask` 加变体 + typed 请求/结果 + 适配器（WS 通道）；选择器只需新调用点。
- **协议漂移**：改单个适配器文件（快照测试兜底）；未发版期间用户以 params 逃生舱自救。

## 8. 风险与开放问题

1. **安全（独立立项）**：api_key 明文出 wire、加密密钥同目录；新连接 API 借机默认打码、只写不回读。
2. **多 key 语义**：基座统一轮换；per-key 健康/冷却持久化列 P4 候选。
3. **v3 数据契约**：新表须登记逻辑关联与删除策略（provider 删除级联连接与模型行）。
4. **凭证形状校验**：auth_scheme ↔ credentials JSON 的 schema 校验要 fail-closed，避免"配了但缺字段"延迟到调用期。
5. **兼容期**：6 map 投影一个 minor 版本；`conversation.model` 旧快照读兼容保留更久。
6. **参数规整器维护成本**："尽力规整 + warnings"而非硬拒绝，防止变成新硬编码陷阱。
7. **异步任务持久化**：JobHandle 是否入库（重启续轮询）——creation 任务行已持久化 remote_task_id，先沿用其模式，invoke 层不自建队列（队列是产品域职责）。

---

## 附录 A：问题 ↔ 方案映射

| 问题（§2.4/§2.5） | 解决机制 |
|---|---|
| 选择器不认标签 | §3.6-1 统一 resolve；§3.4 管线守门 |
| 探针≠运行时 | probe 与 invoke 同管线 |
| 名字含 gemini 误路由 / STT 名字猜协议 | §3.5 (protocol, task) 路由纪律 |
| 国产图像视频必失败 / TTS·v2v 断头 | ark/volc/dashscope 适配器 + 新入口 |
| **非 chat 模态独立域名/凭证无法表达** | **§3.2 连接档案 + 任务→连接解析链** |
| 默认模型不看任务 | resolve_models(task) 接入全部隐式路径 |
| 参数硬透传 | 白名单规整器 + warnings |
| platform 过载 | 平台注册表单点化 |
| 错误降级字符串 | InvokeErrorKind 统一分类 |
| 契约手工同步 / 绑定快照 | ts-rs / 引用化 |
| **依赖方向（工坊 ≠ 能力通道）** | **§3.1 invoke 底层 crate + 依赖红线** |

## 附录 B：关键文件索引

（同 v1，新增）依赖摸底关键点：creation 依赖清单与被依赖（app `services.rs:1065,2320-2332`、gateway `caps_workshop.rs:17`）；抽取接缝（creation `lib.rs:18-34`、`service.rs:32-41,895,1027`）；STT 配置链（shell `routes.rs:162-295`、app `router/state.rs:1877-1891`）；死码判定（`nomifun-mcp/src/session_injection.rs:147-164` 全仓无生产调用方）；providers 单连接现状（`001_v3_baseline.sql:149-175`）；workspace 登记（根 `Cargo.toml:3,10-44`）。

## 附录 C：供应商协议差异矩阵（2026-07 调研，含来源）

> 完整矩阵表（10 供应商 × 6 模态：域名/路径/鉴权/同异步/请求响应形状，含官方文档 URL 与置信度标注）见配套文档 [`2026-07-28-provider-protocol-variance.zh.md`](./2026-07-28-provider-protocol-variance.zh.md)；此处收录十条设计要点结论。

1. **连接配置必须 per-task**：火山 chat/图/视频在 ark 域（Bearer），TTS/ASR 在 openspeech 域（appid/token/cluster 或 X-Api-* 四头，Ark key 不可用）；MiniMax 双平台 key 不互通且需 GroupId。
2. **鉴权 ≥6 种可插拔**：Bearer / `Token` 前缀（Deepgram）/ 自定义头（x-goog-api-key、xi-api-key、火山四头）/ body 内嵌（火山 TTS v1，含 `Bearer;token` 畸形分号头）/ query / SigV4。
3. **异步 submit/poll 无两家一致**：id 字段名（task_id/id/requestId/operation-name 资源路径）、id 位置（路径/query/POST body）、轮询端点（专属 vs DashScope `/api/v1/tasks/{id}`、智谱 `/async-result/{id}` 平台统一）、状态词表 5+ 套（含火山 ASR 状态在响应头 `X-Api-Status-Code`）、DashScope 以 `X-DashScope-Async: enable` 头区分同异步、客户端发号 vs 服务端发号——必须句柄化归一。
4. **结果取回 0~2 跳**且 URL 普遍 24h~2 天过期（OpenAI Sora 另调 content 端点；MiniMax Success→file_id→download_url 三段式；DashScope ASR 结果是指向转写 JSON 的 URL 二跳）——invoke 层提供 materialize，调用方立即转存。
5. **语音类 WS/二进制流是普遍现象**：实时 ASR 几乎全员 WS；TTS 流式三分天下（裸二进制 HTTP / SSE 内嵌编码块 / WS 会话协议）；编码 base64 vs **hex（MiniMax）** vs 自定义二进制帧——通道四分类 + codec 声明。
6. **请求体 ≥4 形状**：JSON / multipart（`image[]` 数组字段名坑）/ 裸二进制上行（Deepgram）/ 非 OpenAI 包裹结构（Gemini instances/parameters、DashScope input/parameters）；火山 seedance 还把参数拼进 prompt 文本（`--resolution 720p`）。
7. **"OpenAI 兼容"在非 chat 模态是残缺子集**：Gemini 兼容层图像仅认 5 参数且静默忽略其余、无 /audio/*；DashScope compatible-mode 音图不全；gpt-image-1 与 dall-e 同端点词表互斥——按 (供应商,任务,模型) 决定兼容层/原生，参数白名单不透传。
8. **图像 b64/url 分歧**在 invoke 层归一为 `ProducedAsset{Bytes|Url}` + materialize。
9. **同端点代际差异需要 per-model params schema**（gpt-image-1 vs dall-e-3；火山 TTS v1 cluster 体系 vs v3 Resource-Id 体系并存）。
10. **追踪 id 语义**：火山 v3 语音/DashScope-WS 客户端发号且 submit/query 复用——JobHandle 支持双发号模式。

主要来源：OpenAI/Gemini/火山/阿里百炼/MiniMax/智谱/Deepgram/ElevenLabs/SiliconFlow/OpenRouter 官方文档（URL 清单见配套文档 [`2026-07-28-provider-protocol-variance.zh.md`](./2026-07-28-provider-protocol-variance.zh.md)；标注※的二手信息接入前需真实调用核实）。
