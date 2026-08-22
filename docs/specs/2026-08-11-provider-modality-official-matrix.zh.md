# 模型供应商 × 模态官方接口核验矩阵（2026-08-11）

> 核验日期：2026-08-11（Asia/Shanghai）。范围是 `MODEL_PLATFORMS` 中的全部预置项，以及 `custom` / `new-api` 两个用户定义入口。本文同时给出当前模型管理的规范架构与供应商官网公开接口矩阵；模型是否对某一账号、地区或订阅开放，仍以该账号实时目录和控制台为准。

## 1. 结论与强制规则

“OpenAI 兼容”通常只表示某一个入口（多为 Chat Completions）兼容，并不表示图像、视频、语音、向量和重排也兼容。仓库不得再用一个统一 `base_url` 和一个统一协议推导所有模态。

配置顺序固定为：

1. 选择供应商（含地区、计费方式或订阅计划）；
2. 先单选模型的主要类型/任务，用它预筛选实时/维护目录候选；
3. 再通过同一个模型输入源搜索并选择目录模型，或自由填写模型 ID；目录只是候选来源，不能成为录入白名单。只有明确选择目录项这一动作才会应用该类型对应的已核验 traits，且不会自动加入目录声明的其他任务；额外任务只能通过“添加其他任务”逐项加入；
4. 后端协议 manifest 按供应商预置和任务给出已注册协议、鉴权、endpoint 描述及推荐值；供应商已有 adapter 只说明该任务存在可用路由，不能作为具体模型支持该任务的证据；
5. 底层仍允许同一模型保存多条 task capability，但每次请求只携带一个 task，并只解析精确的 `(provider_id, model, task)` 能力行；
6. 没有原生适配器时返回明确的 `UnsupportedTask` / `NoAdapter`，绝不回退到 OpenAI 路径“试一下”。

模型目录策略也必须分开：

- 有官方运行时目录的供应商以实时目录为准，并缓存 `verified_at`、来源 URL 和能力元数据；空目录、401/403、地区无权限都必须明确显示目录状态，不能伪装成一次成功的实时查询。少数已明确登记的官网 fallback 也必须带核验日期、来源和 fallback 标记。
- 没有目录 API 的订阅计划才使用官网白名单；白名单必须带核验日期和来源，过期后应阻止发布，而不是永久保留模型。无论目录是否可用，用户仍可保存官网或私有部署实际存在的手工模型 ID。
- 目录返回模型名不等于该模型支持所有任务。任务要来自官方能力字段、不同模态目录或经过核验的模型族规则。
- `vision` 指图像/音频/视频**输入理解**，仍属于 Chat；它不能自动获得图像/音频/视频**生成**任务。

本文状态符号：`✅` 已有本仓库验证路由和 serializer；`⛔` 本轮内置路由主动拒绝，防止误走 OpenAI（UI 仍可把它标为待适配，而不是冒充可用）；`🧩` 官网支持但仍待原生适配；`—` 官网未提供或不适用。状态描述的是本仓库，不是供应商能力上限。

模型管理必须显式区分两个集合：`known` 只表示 2026-08-11 官网已公开该能力，`supported` 才表示当前构建已经注册 endpoint、鉴权和 serializer。`custom` / `new-api` 可作为用户主动选择的逃生舱，但不能反向把预置供应商的 `known` 自动升级成 `supported`。

目录与预置也不能锁死配置。添加模型时，远端目录只在用户明确选择某个目录项后应用该模型的建议任务与 traits，后台刷新或仅输入相同模型 ID 都不得静默套用；目录拉取失败不得阻止手工保存。用户可以为每个已添加任务分别选择已注册协议、连接角色，编辑该任务的 Base URL 覆盖、提交/轮询/内容/实时 endpoint、上下文限制、traits 和协议专属 JSON 参数。向异源覆盖地址发送凭据必须额外显式启用 `allow_cross_origin_credentials`；请求 serializer 始终由后端已注册协议拥有，不能用任意 JSON 形状冒充协议适配。

## 2. 现行单一能力源架构与九模态配置契约

本节是模型管理的**现行规范**；后续供应商表是 2026-08-11 的官网核验快照。实现不得从供应商名、模型名或其他旧配置形状推断运行协议。

### 2.1 存储与运行时单一事实源

| 实体 | 现行职责 |
|---|---|
| `providers` | 供应商身份、启停状态、配置版本，以及默认连接的根 `base_url`、显式 `auth_scheme` 和加密 typed credentials。响应只返回 `has_credentials`，绝不回显秘密。默认连接不表示所有模态共用相同 endpoint。 |
| `provider_connections` | 同一供应商的命名连接；每条连接独立保存角色、根 Base URL、鉴权方案和加密 typed credentials，用于语音专线、地区站点或其他独立产品面。响应同样只暴露 `has_credentials`。 |
| `provider_models` | 只保存模型身份与展示字段：`provider_id`、自由填写的 `model` ID、启停、排序、描述和时间戳。 |
| `provider_model_capabilities` | 唯一键为 `(provider_id, model, task)`；每个模型 × 模态一行，保存 `traits`、`protocol`、`connection_role`、`base_url_override`、四类 typed endpoint、跨域凭据确认、`provider_params`、上下文限制及该模态自己的健康状态。行存在表示该模态已配置。 |

供应商和模型必须启用，且请求任务必须存在精确 capability 行，运行时才允许解析。运行时从该行取得协议、连接角色、transport 覆盖和供应商参数；缺行返回 `UnsupportedTask`，协议未注册或与任务不匹配返回配置错误，不做供应商默认协议或模型名 fallback。每次会影响调用图的供应商、连接或 capability 修改都会原子递增 `providers.config_revision`；解析器必须读取一致版本，异步 `JobHandle` 和健康结果也必须绑定该版本，旧配置产生的轮询或探测结果不得写回新配置。数据库迁移 032 只负责一次性把历史配置物化到新表，迁移完成后不再参与运行时解析。

凭据只使用一套 typed JSON 契约：普通密钥为 `{ "api_keys": ["..."] }`，Bedrock access-key 模式为 `{ "access_key_id": "...", "secret_access_key": "...", "session_token": "..." }`，Profile / DefaultChain 使用空对象。数据库只保存统一的 `credentials_encrypted`。这是一次明确的破坏性收敛：迁移会清除旧 default/named connection 密文并剥离旧 Bedrock/connection JSON 中的秘密字段，升级后需要用户重新录入凭据；仓库不保留旧字段双读、密文复制或响应回显通道。

### 2.2 九种任务彼此独立

`MODEL_TASK_ORDER` 是任务选择器的完整选项顺序，不表示把供应商全部已适配任务展示成模型能力。一个模型仍可保存多个任务，且每个任务都可独立删除和配置，不会因为目录声明了另一任务而自动继承协议或 endpoint。目录候选只显示明确包含所选首个任务的模型；手填模型也只初始化已声明的任务。

> **2026-08-22 修订**：本节与 §2.4 第 4 条原先要求"先单选主类型"并禁止"九项自由多选"。该约束已按产品决定撤销：任务选择器现在是多选控件，已声明的任务以标签形式显示在控件内。
>
> 撤销原因：所谓"主类型"在数据层不存在——它只是 `capabilities[0].task`，既无 draft 字段也无 `provider_model_capabilities` 列，且读取时后端会按 task 重排（`provider_model.rs` `row_to_model_response`），因此用户选的"主类型"活不过一次重载。它实际只承担两件事：给模型目录一个筛选键、以及在选完之前锁住模型 ID 输入框——两者都可以由"已声明任务中的第一个"承担。单选控件本身还引入过一个真实缺陷：选完后控件回到 placeholder，用户无法确认选择已生效。
>
> 原禁令要防的风险仍然成立且仍被拦住：勾选一个任务就会生成一张待配置的能力卡，缺协议时保存会被 `protocol_required` 拒绝并明确指出缺什么；从标签 `×` 移除一个**已配置**的任务会先弹确认，因为移除任务等于删掉它的协议与地址（`capabilityHasConfiguration`）。目录声明多任务也仍然不会自动多选——`applyCatalogSuggestionForTask` 只处理被选中的那一个任务。

| `task` | 产品含义 | 常见 typed 路由字段；实际显示项由 manifest 决定 |
|---|---|---|
| `chat` | 文本对话及图片/音频/视频输入理解 | `endpoint`；同步 JSON 或 SSE 等传输由协议定义 |
| `realtime_conversation` | 双向实时会话 | `realtime_endpoint`；持久 WebSocket/session 协议 |
| `image_generation` | 图像生成 | `endpoint`，异步协议可另有 `poll_endpoint` / `content_endpoint` |
| `image_edit` | 图像编辑 | 独立 `endpoint`；不得从图像生成能力自动获得 |
| `video_generation` | 视频生成 | `endpoint` + 按协议需要的 `poll_endpoint` / `content_endpoint` |
| `speech_synthesis` | TTS | `endpoint`，流式协议可声明 `realtime_endpoint` |
| `speech_recognition` | ASR | `endpoint`，批处理可声明 `poll_endpoint`，流式协议可声明 `realtime_endpoint` |
| `embedding` | 向量化 | `endpoint` |
| `rerank` | 重排 | 独立 `endpoint`；不得从 Embedding 能力自动获得 |

`traits` 只是某一 capability 内的输入/输出特征，例如 `vision_input`、`audio_input` 或 `streaming`；它不能替代上述九种任务，也不能创建额外运行路由。

九种任务不能只停留在“可保存、可探活”。当前产品消费者包括：Chat 会话，Workshop 的图像生成/编辑、视频生成与 TTS，聊天/机器人 ASR，机器人 TTS，以及知识库检索的 Embedding / Rerank。知识库通过 `knowledge.retrieval` 保存两个彼此独立的 tagged stage：每个 stage 都明确选择 `local`，或选择精确的 `{provider_id, model}` remote capability。remote Embedding 在查询时批量计算查询与有界候选文档向量并按余弦相似度排序；remote Rerank 可独立接在本地词法候选或远程向量候选之后。配置错误、上游错误、向量维度/数量错误、重排索引错误或资源预算超限都会明确失败，不会静默回落 local。

当前 Markdown 文件仍是知识库唯一内容真源；本轮没有伪造持久 chunk/vector 索引。远程 Embedding 是有显式文档数、单文档摘要长度和总字符预算的 query-time 语义检索。超过预算时要求用户缩小知识库范围，而不是按路径静默只搜索前一部分文档。若未来引入持久向量索引，必须作为完整独立功能同时解决分块、模型/维度版本、增删改失效、重建和迁移，不能与本契约双写。

### 2.3 后端协议 manifest 与可编辑 transport

后端 `nomifun-model-invoke` 的协议 registry 是协议、鉴权和 endpoint 描述的唯一来源。前端通过：

```text
GET /api/model-protocols?preset=<preset>&task=<task>&base_url=<optional>&model=<optional>
```

读取生成的 `ModelProtocolManifestResponse`；编辑已有供应商时也可用 `platform` 查询参数。响应包含该任务已注册的协议、executor/transport、允许的鉴权方案、推荐连接与 Base URL，以及 `endpoint`、`poll_endpoint`、`content_endpoint`、`realtime_endpoint` 的默认值和可编辑性。任务卡必须展示 manifest 返回的全部供应商推荐、已核验和通用高级协议，只高亮推荐项，不能把推荐项当白名单；前端也不得维护另一份协议清单。

`model` 只是配置期“用户已经选择或填写了具体模型”的可选提示，不得解析模型名称来识别厂商或协议。仅当解析后的 `platform` **严格等于** `custom` 且 `model` 非空时，manifest 才可从 protocol registry 推荐当前任务的通用兼容协议：候选 descriptor 必须同时声明该 `task`、`official_compat` 和 `custom` scope，并且候选必须恰好一个；零个或多个都不推荐。`new-api` 不适用此默认策略，Realtime 在没有唯一通用兼容协议时同样保持无默认值。该推荐不得携带原厂 Base URL、命名连接或原厂鉴权；`connection_role` 不指定命名连接、`default_base_url` 为空，鉴权最多沿用 `custom` 预置且必须在协议允许列表内。

每个 capability 的 transport 字段语义固定为：

| 字段 | 语义 |
|---|---|
| `base_url_override` | 仅覆盖当前任务所选连接的 Base URL；用于同一供应商不同模态的独立域名或根路径。 |
| `endpoint` | 提交或同步调用地址。省略时只能使用所选协议 descriptor 的注册默认值。 |
| `poll_endpoint` | 异步任务状态查询地址。 |
| `content_endpoint` | 异步产物下载或物化地址。 |
| `realtime_endpoint` | WebSocket 或其他持久实时会话地址。 |
| `allow_cross_origin_credentials` | 当任一绝对覆盖 URL 与凭据所属连接异源时，必须由用户显式确认。 |
| `provider_params` | 协议专属 JSON 参数；保存校验与运行时使用同一编码契约。JSON 协议递归合并后由 typed 请求字段最终覆盖；multipart/query 只接受能无损编码的值及协议明确声明的数组形式；不能发送的值在保存时直接报错，不能“保存成功、调用时静默丢弃”，也不能携带 transport 或凭据字段。 |

协议切换是一次原子编辑：必须清除旧协议的 endpoint 覆盖、跨域确认和供应商参数，再应用新 manifest 的推荐值，防止参数泄漏到错误 serializer。包括 `custom` 自动推荐在内，推荐结果只有在保存 capability 时被**显式持久化**后才成为调用配置；运行时和健康探针只读取精确 capability 行，绝不再次调用配置期推荐、解析模型名或猜测协议。manifest 和供应商 adapter 状态只能描述一个已选任务能否路由，不得据此为模型新增任务或宣称模型具备该能力。manifest 没有已注册协议时，UI 仍展示该任务和“官网已知/暂无适配器”状态，但不能把它标成可运行。

### 2.4 配置与写入流程

新增供应商、给已有供应商添加模型和编辑模型统一复用 `ModelDefinitionEditor`，不再各自维护一套模态或协议表单。

1. 选择供应商预置、地区或计划，并填写默认连接的 Base URL、鉴权和凭据；需要独立产品面时可内联创建任意命名连接。
2. 先单选模型的主要类型/任务，并只展示目录中明确包含该任务的候选；taskless 目录项不视为支持全部类型。
3. 在同一个模型输入源里搜索并选择目录模型，或自由填写模型 ID；目录加载失败或没有匹配项不得阻止手工输入和保存。明确选择目录项时，只应用主任务对应的已核验 traits；仅输入相同 ID、后台目录刷新或供应商 adapter 状态都不得静默改变任务集合。
4. 任务通过多选控件声明，已声明的任务以标签形式显示在控件内（2026-08-22 修订，见 §2.2）。仍不得因目录模型声明多任务而自动多选；从标签移除一个已配置任务前必须确认，因为那会连带丢弃它的协议与地址。
5. 在每个任务卡中选择 manifest 注册协议，并分别编辑实际 Base URL、typed endpoint、连接角色、鉴权兼容性、上下文限制和协议专属参数。`custom` 在模型 ID 非空且 registry 存在唯一通用兼容候选时可预选该协议，用户无需展开高级配置手工填写；保存时仍将协议 ID 与 transport 配置显式写入该 capability。
6. 新建供应商通过 `POST /api/providers` 一次提交供应商、`initial_model` 和所需命名连接，数据库原子创建完整能力图；已有供应商使用 `PUT /api/provider-models` 的 `SaveProviderModelRequest` 全量保存一个模型并原子替换其 capability 集合。模型配置在响应中只以嵌套的 `models[].capabilities[]` 形状返回。

主类型单选和额外任务添加器只是新建交互约束，不改变底层多 task 契约。编辑已有多任务模型时必须完整加载并保留其 capability 集合；运行时每次请求仍只选择一个 task，并只按精确的 `(provider_id, model, task)` 行取得协议和 transport 配置。

因此，`known` 与 `supported` 的边界不会被自定义能力绕过：官网矩阵可以说明供应商存在某能力，但只有 manifest 中注册且成功保存的协议才进入运行选择器。`custom` / `new-api` 允许使用 manifest 已登记的通用高级协议和用户自有 URL/连接，仍不能提交未注册协议字符串。

## 3. 供应商、地区、计划与目录总表

| `platform` / 预置项 | 官方根地址与鉴权边界 | 当前目录策略 | 生命周期与仓库策略 |
|---|---|---|---|
| `custom` | 用户给定；鉴权、协议和完整 endpoint 都是用户数据 | 不猜测；可手工模型，或显式配置目录 URL/解析器 | 配置期可按已选任务预选 registry 中唯一的通用兼容协议，并在保存时显式持久化；不解析模型名称、不注入原厂连接/URL，运行时不猜协议。无唯一安全候选时仍由用户选择。 |
| `new-api` | 用户网关根；每个部署启用的上游不同 | `GET {base}/v1/models`，但仅能证明网关公开了 ID | 每模型显式 `openai` / `anthropic` / `gemini`；非 Chat 模态只有网关声明且仓库有对应协议适配器时才展示。 |
| `openai` | `https://api.openai.com/v1`，Bearer | 动态 `GET /models`，再按官方模型页/endpoint 能力分组 | 不保存“永久可用”静态列表；下架与弃用以 [Models](https://developers.openai.com/api/docs/models/all) 和官方弃用信息为准。 |
| `anthropic` | `https://api.anthropic.com`；`x-api-key` + `anthropic-version` | 动态 `GET /v1/models` | 仓库旧 fallback 中 `claude-3-opus-20240229`、`claude-3-sonnet-20240229`、`claude-3-haiku-20240307` 已不可继续作为当前默认；按 [Models](https://docs.anthropic.com/en/docs/about-claude/models/overview) 与 [deprecations](https://docs.anthropic.com/en/docs/resources/model-deprecations) 更新。仅 Chat ✅。 |
| `bedrock` | 控制面 `https://bedrock.{region}.amazonaws.com`、Native `bedrock-runtime`、OpenAI/Anthropic 兼容面 `bedrock-mantle`；AWS SigV4，不是 API Key base。鉴权显式区分 AccessKey（含可选 STS session token）、Profile、DefaultChain。 | 合并 `ListFoundationModels` 与分页 `ListInferenceProfiles`，识别区域前缀和 backing model ARN；模型 ID、profile、区域和账号权限共同决定可用性。目录只作建议，手填始终可用。 | 同一模型可能只支持 Converse、原生 Invoke、Mantle、AsyncInvoke、Sonic 双向流或 agent-runtime rerank 中一部分。仓库当前 canonical Chat 协议为 `bedrock.anthropic_messages`，仅实现 Claude `invoke-with-response-stream`，不冒充通用 Converse；非 Anthropic 目录项不自动获得 Chat capability。除已验证 Claude Chat 外均 ⛔/🧩。以 [model availability](https://docs.aws.amazon.com/bedrock/latest/userguide/models.html) 为准。 |
| `gemini` | `https://generativelanguage.googleapis.com`；`x-goog-api-key`，稳定接口用 `v1`、预览接口用 `v1beta` | 动态 `GET /v1beta/models`，读取 `supportedGenerationMethods` | 当前 Chat 主线为 Gemini 3.6/3.5/3.1；Gemini 2.0 已于 2026-06-01 shutdown。`*-preview`、实验 ID 和别名按官方 [models](https://ai.google.dev/gemini-api/docs/models) / deprecation 表更新，不能作为永久 fallback。Chat、图像生成/编辑 ✅；其他官网能力 🧩。 |
| `gemini-vertex-ai` | `https://{location}-aiplatform.googleapis.com/v1/projects/{project}/locations/{location}`；OAuth2/ADC | Google publisher model + 项目/区域可用性；不能复用 Gemini Developer API 的 key 与目录 | 必须拆成 `publishers/google` 的 Gemini 与 `publishers/anthropic` 的 Claude。旧预置会把 UI 的 Gemini 2.5 发往 Anthropic `streamRawPredict`，属于确定性错配；本轮已将它从“新建供应商”列表移除 ⛔，待拆分成两个正确产品面后再恢复。预览 Veo endpoint 迁移见 [Vertex release notes](https://cloud.google.com/vertex-ai/docs/release-notes)。 |
| `deepseek` | `https://api.deepseek.com`（`/v1` 兼容别名可用），Bearer | 官方当前目录/更新页；无目录时只允许核验白名单 | 2026-07-24 后 `deepseek-chat`、`deepseek-reasoner` 已退役；当前为 `deepseek-v4-flash`、`deepseek-v4-pro`。仅文本 Chat/Responses；见 [updates](https://api-docs.deepseek.com/updates)。Chat ✅。 |
| `deepgram` | `https://api.deepgram.com`；项目 key 使用 `Authorization: Token ...`（短期 JWT 可用 Bearer） | 原生 `GET /v1/models`，分别读取 `stt[]` / `tts[]`，以 `canonical_name` 保存模型并保留来源任务；不能按名称猜模态 | 预录音 ASR `deepgram.listen` 与 REST TTS `deepgram.speak_rest` 已原生适配 ✅；流式 Listen/Speak 和 Voice Agent 的 WebSocket 是不同协议，其中双向 Voice Agent 属于官网 `known`、当前仍 🧩。见 [模型目录](https://developers.deepgram.com/reference/manage/models/list)、[Listen](https://developers.deepgram.com/reference/speech-to-text/listen-pre-recorded)、[Speak](https://developers.deepgram.com/reference/text-to-speech/speak-request?explorer=true) 与 [Voice Agent](https://developers.deepgram.com/reference/voice-agent/voice-agent)。 |
| `mimo` | `https://api.xiaomimimo.com/v1`；`api-key` 或 Bearer | 动态 `GET /models`；截至核验日精确返回 6 个 v2.5 ID | 当前：`mimo-v2.5-pro`、`mimo-v2.5`、`mimo-v2.5-asr`、`mimo-v2.5-tts`、`mimo-v2.5-tts-voicedesign`、`mimo-v2.5-tts-voiceclone`，见 [official list](https://mimo.mi.com/docs/en-US/api/model/list-models)。旧 `mimo-v2-pro`、`mimo-v2-omni`、`mimo-v2-flash`、旧 `mimo-v2-tts` 于 2026-06-30 退役；本轮 Chat、ASR、TTS 均已按 `/chat/completions` 的模型专属序列化原生适配 ✅。 |
| `mimo-token-plan-cn` / `sgp` / `ams` | `https://token-plan-cn.xiaomimimo.com/v1`、`https://token-plan-sgp.xiaomimimo.com/v1`、`https://token-plan-ams.xiaomimimo.com/v1`；`tp-` key | 计划白名单/计划端目录，按区域分别缓存 | 计划 key 与按量 `sk-` key 不互通，且计划仅授权官方 coding tools，不应作为通用后端推理入口；仅 Chat ✅。 |
| `minimax` | 中国 `https://api.minimaxi.com/v1`；Bearer | 官网跨模态模型页；没有单一、可依赖的全模态目录时使用按模态核验白名单 | 当前文本主线含 `MiniMax-M3`、`MiniMax-M2.7`、`MiniMax-M2.7-highspeed`；旧 `MiniMax-Text-01`、abab 别名不得 fallback。当前接口不再强制 `GroupId`，仓库不能因缺少 GroupId 拒绝。Chat/TTS ✅；图像/视频 🧩。 |
| `minimax-code` | 国际 `https://api.minimax.io/v1`；国际 key | 与中国站分别维护官网白名单 | 中国/国际 key 不互通；当前仓库仅 Chat ✅。 |
| `minimax-coding-plan` | 中国计划 gateway；计划 key | 静态计划 allowlist：`MiniMax-M3`、`MiniMax-M2.7`、`MiniMax-M2.7-highspeed` | 计划只展示 Chat；其他模态 ⛔。来源见 [MiniMax API overview](https://platform.minimaxi.com/docs/api-reference/api-overview)。 |
| `novita` | LLM：`https://api.novita.ai/openai/v1`；媒体：`https://api.novita.ai/v3`，Bearer | LLM `/models` 与媒体 `/v3/model`/各产品模型目录分开 | “OpenAI”根不能调用 v3 图像/视频任务。Chat/Embedding ✅；图像、视频、音频官网可用但 🧩。见 [Novita model API](https://novita.ai/docs/api-reference/model-apis-get-model)。 |
| `openrouter` | `https://openrouter.ai/api/v1`，Bearer | 动态 `/models`；图像还可用 `/images/models`，必须读 `architecture.input_modalities` / `output_modalities` | 不能把所有聚合模型都标成所有模态。Chat/Embedding ✅；图像/视频/音频 ⛔/🧩。见 [multimodal overview](https://openrouter.ai/docs/guides/overview/multimodal/overview)。 |
| `dashscope` | 北京：兼容 `https://dashscope.aliyuncs.com/compatible-mode/v1`，原生 `https://dashscope.aliyuncs.com/api/v1`，WS `wss://dashscope.aliyuncs.com/api-ws/v1/inference`；新加坡/美国用 `-intl` / `-us` 独立域名 | 兼容 `/models` + [官方模型目录](https://help.aliyun.com/en/model-studio/models)，按地域过滤 | 兼容根与原生根不能拼接；`gte-rerank` 已于 2026-05-30 下线，更多模型按 [deprecation](https://help.aliyun.com/en/model-studio/model-depreciation) 处理。Chat、图像生成、Embedding ✅；其余 🧩。 |
| `dashscope-coding` | OpenAI：`https://coding.dashscope.aliyuncs.com/v1`；Anthropic：`https://coding.dashscope.aliyuncs.com/apps/anthropic` | 官网静态 allowlist | 当前精确白名单：`qwen3.7-plus`、`qwen3.6-plus`、`kimi-k2.5`、`glm-5`、`MiniMax-M2.5`、`qwen3.5-plus`、`qwen3-max-2026-01-23`、`qwen3-coder-next`、`qwen3-coder-plus`、`glm-4.7`；见 [Coding Plan](https://help.aliyun.com/en/model-studio/coding-plan)。仅 Chat ✅。 |
| `siliconflow`（CN / Global） | `https://api.siliconflow.cn/v1` / `https://api.siliconflow.com/v1`，Bearer；账号与模型可用区分站 | 动态 `/models`；官网支持 `type` / `sub_type` 过滤，但当前目录仍只是 ID 建议，不能据此给每个模型自动授予所有任务，音频模型也可手工录入 | 图像响应、视频 submit/status、音频字段均非“只换 base”的 OpenAI 等价物。本轮 Chat、图像生成/编辑、视频 submit/status、TTS、ASR、Embedding 与同步 Rerank 均已适配 ✅；TTS 使用原生 `siliconflow.audio_speech`、供应商音色 ID 和二进制音频响应。见 [Create speech](https://docs.siliconflow.com/en/api-reference/audio/create-speech) 与 [List models](https://docs.siliconflow.com/en/api-reference/models/get-model-list)。 |
| `zhipu` | 中国 `https://open.bigmodel.cn/api/paas/v4`，Bearer | 官方 OpenAPI 没有公共 `GET /models`，因此使用按模态、带来源和日期的维护目录 | 当前目录覆盖文本、视觉理解、`glm-4-voice`、`glm-image`/CogView、已核准精确 ID 的 CogVideo、`glm-asr-2512`、`glm-tts`、`embedding-2/3`、`rerank`；Vidu 虽共用视频协议，但在精确 callable ID 未全部进入总览前不猜别名。本轮仅 Chat、视频异步任务、Embedding、Rerank 有验证路由 ✅；图像、ASR、TTS 仍为官网 `known` 但内置路由 ⛔/🧩，不得复用会固定注入 OpenAI 字段的 serializer。见 [模型总览](https://docs.bigmodel.cn/cn/guide/start/model-overview)、[视频生成](https://docs.bigmodel.cn/api-reference/模型-api/视频生成异步) 与 [查询异步结果](https://docs.bigmodel.cn/api-reference/模型-api/查询异步结果)。 |
| `glm-coding-plan` | `https://open.bigmodel.cn/api/coding/paas/v4`，计划 key | 静态 allowlist：`glm-5.2`、`glm-5-turbo`、`glm-4.7` | 计划仅 Chat ✅；不可带出图像/视频任务。 |
| `moonshot-cn` / `moonshot-global` | `https://api.moonshot.cn/v1` / `https://api.moonshot.ai/v1`，Bearer | 动态 `/models`，中外站分别缓存 | 当前主线为 K3 / K2.7 / K2.6；`kimi-k2.5` 与 `moonshot-v1` 已公告 2026-08-31 下线，不能长期 fallback。Chat（含视觉输入模型）✅；生成类模态 —。见 [Moonshot API docs](https://platform.moonshot.cn/docs/api/chat)。 |
| `xai` | `https://api.x.ai/v1`，Bearer | `GET /models` 只有最小字段；按模态合并 `/language-models`、`/image-generation-models`、`/video-generation-models` 的完整 metadata；TTS/STT 是无 `model` 字段的 service profile | 图像编辑是 JSON `POST /images/edits`，视频是异步 `/videos/generations` + request 查询，TTS/STT 是 `/tts`、`/stt`，均不是 OpenAI Audio/Video 路径。本轮 Chat、图像生成/编辑、视频、TTS、STT 均已用原生 profile ✅；Embedding/Rerank —。见 [xAI models](https://docs.x.ai/developers/models) 与 [modality model catalogs](https://docs.x.ai/developers/rest-api-reference/inference/models)。 |
| `ark` | `https://ark.cn-beijing.volces.com/api/v3`，Bearer；语音另用 `openspeech.bytedance.com` 一族 endpoint/凭证 | 普通 Ark 使用账号 endpoint/model ID；按控制台/官方模型清单同步 | Chat、图像、视频 ✅；TTS/ASR ✅ 但必须使用独立连接角色、域名和 `appid/token/cluster` 或 `X-Api-*`，不可拿 Ark Bearer key 调语音。见 [Ark docs](https://www.volcengine.com/docs/82379)。 |
| `ark-coding-plan` | `https://ark.cn-beijing.volces.com/api/coding/v3` | 静态 `ark-code-latest` | 计划仅 Chat ✅。 |
| `ark-agent-plan` | `https://ark.cn-beijing.volces.com/api/plan/v3` | 优先计划端目录；不可用时仅用官网核验的计划白名单 | 路由别名 `ark-code-latest` 与计划允许的具体模型只用于 Chat；所有媒体 ⛔。 |
| `qianfan` | `https://qianfan.baidubce.com/v2`，Bearer；视频为同域 `/video`，语音属于百度语音独立产品/域名 | 动态 `GET /v2/models` | Chat、Embedding、同步 Rerank ✅；图像/编辑/视频/语音虽官网支持但需原生适配，当前 ⛔/🧩。见 [千帆 v2 API](https://cloud.baidu.com/doc/qianfan/s/rmh4stp0j)。 |
| `qianfan-coding-plan` | `https://qianfan.baidubce.com/v2/coding` | 静态官网 allowlist | 当前：`qianfan-code-latest`、`kimi-k2.5`、`deepseek-v3.2`、`glm-5`、`minimax-m2.5`、`ernie-4.5-turbo-20260402`、`deepseek-v4-flash`、`glm-5.1`。不得混入 DashScope 的 Qwen 列表；仅 Chat ✅。 |
| `hunyuan` | 旧 `https://api.hunyuan.cloud.tencent.com/v1`；新 TokenHub 中国 `https://tokenhub.tencentmaas.com/v1`、国际 `https://tokenhub-intl.tencentmaas.com/v1`（腾讯国际站另有 `tencentcloudmaas.com` 域名） | 新 TokenHub `GET /models`，读取 `online` / `pre-offline`，并按站点、地域隔离凭证 | 旧平台 2026-06-30 起停止售卖/新建资源，并于 2026-09-30 整体停服；`hy3-preview` 2026-08-31 下线，`hy-image-v3.0`/`hy-image-lite` 2026-09-15 停止发起任务。本轮已新增中国/Global TokenHub 预置并停止把旧平台作为新建默认；当前通用 Chat ✅，其他 TokenHub 原生任务 ⛔/🧩。见 [旧平台公告](https://cloud.tencent.com/announce/detail/2287) 与 [TokenHub API](https://cloud.tencent.com/document/product/1823/130078)。 |
| `lingyi` | `https://api.lingyiwanwu.com/v1`，Bearer | 优先 `/models`；无目录时仅当前官网白名单 | 当前核验 `yi-lightning`、`yi-vision-v2`，其中 vision 是图像输入理解，不是图像生成。仅 Chat ✅；见 [零一万物文档](https://platform.lingyiwanwu.com/docs)。 |
| `poe` | `https://api.poe.com/v1`，Bearer | 动态 `GET /models`，读取 `architecture` 与 input/output modalities | Poe 的 bot 目录变化快；必须按 bot 能力过滤。Chat ✅；专用图像/视频/语音接口仍 🧩。见 [OpenAI-compatible API](https://creator.poe.com/docs/external-applications/openai-compatible-api)。 |
| `ppio` | LLM `https://api.ppio.com/openai/v1`；媒体 `https://api.ppio.com/v3`，Bearer | LLM `/models` 与媒体模型页分开 | 旧 `api.ppinfra.com/v3/openai` 已过时；通用 `/v3/async/txt2video` 与 `/v3/async/img2video` 于 2025-07-31 退役，旧通用图像接口于 2026-01-31 退役，媒体必须走当前模型专用 endpoint。Chat/Embedding/Rerank ✅；媒体 ⛔/🧩。见 [PPIO API docs](https://ppio.com/docs/model-api/)。 |
| `modelscope` | 中国 `https://api-inference.modelscope.cn/v1`（国际资料另见 `.ai`），Bearer token | API-Inference 在线模型目录/模型卡；Hub `/models` 不是运行时可调用目录的同义词 | 当前仓库只承诺 OpenAI-compatible Chat/VLM ✅；视觉输入仍是 Chat。其他 Hub 任务类型不等于已经有在线 API。见 [API-Inference](https://modelscope.cn/docs/model-service/API-Inference/intro)。 |
| `infiniai` | `https://cloud.infini-ai.com/maas/v1`，Bearer | 动态 `/models`，按在线状态/能力字段筛选 | Chat、Embedding ✅；Rerank 等官网能力在原生适配完成前 ⛔。见 [InfiniAI docs](https://docs.infini-ai.com/)。 |
| `ctyun` | TokenHub `https://ai.ctaigw.cn/v1`，Bearer AppKey；星辰 MaaS 语音使用独立 WS path、`X-APP-ID` 与签名鉴权 | TokenHub 原生 `GET /models`/控制台在线服务目录；按接口类型过滤。独立语音产品不能从该目录或根地址推导 | 旧 `wishub-x1` 文档已停止维护；现行通用接口表列出 Chat/VLM、Embedding、Rerank，Image 由同版独立页面记录在 `ai.ctaigw.cn`，本轮四类均已适配 ✅。官网另有 WS TTS/ASR，代码只将其记为 `known`，因域名/鉴权/session serializer 尚未适配而仍 🧩；精确命中旧 x6 预置的已有 TokenHub 配置由一次性迁移 032 更新。见 [接口类型列表](https://www.ctyun.cn/document/11061839/11062345)、[Image API](https://www.ctyun.cn/document/11061839/11062322)、[模型列表](https://www.ctyun.cn/document/11061839/11062357)、[WS TTS](https://www.ctyun.cn/document/11092117/11093798) 与 [WS ASR](https://www.ctyun.cn/document/11092117/11093804)。 |
| `stepfun` | `https://api.stepfun.com/v1`，Bearer；Anthropic SDK base 是 `https://api.stepfun.com`；Realtime 为独立 WS | 动态 `/models`；仅精确官网根临时不可用时允许使用带核验日期的公开模型 fallback，未知未来 ID 原样保留 | 本轮 Chat、`/images/generations`、`/images/edits`、HTTP TTS 与推荐 `stepaudio-2.5-asr` 的 JSON+SSE 协议已有验证路由/原生 serializer ✅；官网 `/images/image2image`、`style_reference` 等未覆盖高级字段及独立流式 TTS WS 仍 🧩。双向 Realtime 后端 session/健康探测 ✅，但用户产品会话桥仍 🧩，因此 UI `supported` 暂不包含 Realtime，只保留官网 `known`。见 [HTTP TTS](https://platform.stepfun.com/docs/zh/api-reference/audio/create-audio)、[WS 流式 TTS](https://platform.stepfun.com/docs/zh/api-reference/audio/ws-audio)、[SSE ASR](https://platform.stepfun.com/docs/zh/api-reference/audio/asr-sse)、[双向实时语音](https://platform.stepfun.com/docs/zh/api-reference/realtime/chat) 与 [图像编辑](https://platform.stepfun.com/docs/zh/api-reference/images/edits)。 |
| `stepfun-plan` | OpenAI `https://api.stepfun.com/step_plan/v1`；Anthropic SDK 根 `https://api.stepfun.com/step_plan`；Realtime `wss://api.stepfun.com/step_plan/v1/realtime` | 无官方 `/models`；官网当前静态 allowlist 共 9 个：`step-3.7-flash`、`step-3.5-flash`、`step-3.5-flash-2603`、`stepaudio-2.5-realtime`、`stepaudio-2.5-chat`、`stepaudio-2.5-tts`、`stepaudio-2.5-asr`、`step-router-v1`、`step-image-edit-2` | `step-router-v1` 是 Chat 路由，不接受图片/文档；本轮 Chat、图像 generation/edit task、原生 HTTP TTS 与 JSON+SSE ASR 已适配 ✅，图像协议边界同普通版，独立流式 TTS WS 仍 🧩。Realtime 后端使用独立持久会话 registry ✅，不会被当作 Chat 或一次性 HTTP 请求；用户产品桥仍 🧩，UI 同样只把 Realtime 记为 `known`。见 [Step Plan overview](https://platform.stepfun.com/docs/zh/step-plan/overview) 与 [语音模型接入](https://platform.stepfun.com/docs/zh/step-plan/integrations/audio-api)。 |

## 4. 每模态 endpoint / 协议 / 同异步矩阵

下表列出供应商公开能力。为便于横向阅读，九种任务在列上合并为五组展示，但存储、UI 和运行时仍按 §2.2 的九个 task 分行处理；图像生成与编辑、TTS 与 ASR、Embedding 与 Rerank 之间都不继承配置。`输入` 表示将图片/音频/视频放入 Chat 内容；不产生媒体文件。省略的 endpoint 不是“沿用 OpenAI”，而是该预置当前不应暴露该任务。

| 供应商族 | Chat / 多模态理解 | 图像生成 / 编辑 | 视频生成 | TTS / ASR | Embedding / Rerank |
|---|---|---|---|---|---|
| OpenAI | `POST /responses` 或 `/chat/completions`；JSON；同步或 SSE | `POST /images/generations`；编辑 `POST /images/edits` multipart；同步任务响应 | `POST /videos`，`GET /videos/{id}` 轮询，`GET /videos/{id}/content`；异步 | `/audio/speech` 返回音频流；`/audio/transcriptions` multipart；同步/流式依模型 | `/embeddings` 同步；无通用 rerank。官方：[Images](https://developers.openai.com/api/docs/guides/image-generation)、[Video](https://developers.openai.com/api/docs/guides/video-generation)、[Audio](https://developers.openai.com/api/docs/guides/text-to-speech)。 |
| Anthropic | `POST /v1/messages`，JSON，同步/SSE；图片/PDF是输入内容块 | — | — | — | — |
| Bedrock | `Converse/ConverseStream` 或 `POST /model/{modelId}/invoke[...stream]`，SigV4；body 由模型族决定 | 同一 InvokeModel 路径但 body/响应由图像模型决定 | 模型专属异步 invoke/job；不能假设统一 poll | 模型专属 InvokeModel/流；不能复用 OpenAI Audio | embedding 走模型专属 InvokeModel；不是统一 `/embeddings`。官方：[InvokeModel](https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_InvokeModel.html)。 |
| Gemini Developer API | `POST /v1beta/models/{model}:generateContent` 或 `:streamGenerateContent?alt=sse`；内容为 `contents.parts` | Gemini image-output 走 `generateContent`；Imagen 使用专用预测协议；编辑仍是内容 parts，不是 OpenAI multipart | Veo long-running operation，提交后按 operation name 轮询 | TTS 可由 `generateContent` 请求 audio 输出；Live/实时走 WebSocket；ASR 多为理解输入 | `models/{model}:embedContent` / `batchEmbedContents`；无通用 rerank。官方：[API reference](https://ai.google.dev/api)。 |
| Vertex AI | `.../publishers/google/models/{model}:generateContent` / `:streamGenerateContent`，OAuth | Imagen `:predict`，请求为 `instances/parameters` | Veo `:predictLongRunning` + operation 轮询 | Live API/模型专用 endpoint | `:predict` 或 `:embedContent`（依模型）；均受 project/location 限制。 |
| DeepSeek | `/chat/completions`；另有 `/responses`（Flash）和 `/beta/completions` FIM（Pro）；JSON/SSE | — | — | — | — |
| Deepgram | Voice Agent 是独立双向 WebSocket 会话，不是 Chat Completions，当前 🧩 | — | — | 预录音 ASR：`POST /v1/listen`，官网接受原始音频或 URL JSON，当前 `deepgram.listen` 适配原始音频 ✅；REST TTS：`POST /v1/speak?model=...`，JSON 文本、二进制音频，`deepgram.speak_rest` ✅。流式 Listen/Speak 与 `wss://agent.deepgram.com/v1/agent/converse` Voice Agent 仍 🧩 | —。官方：[Models](https://developers.deepgram.com/reference/manage/models/list)、[Listen](https://developers.deepgram.com/reference/speech-to-text/listen-pre-recorded)、[Speak](https://developers.deepgram.com/reference/text-to-speech/speak-request?explorer=true)、[Voice Agent](https://developers.deepgram.com/reference/voice-agent/voice-agent)。 |
| MiMo | `/chat/completions`；`mimo-v2.5` 的图片/音频/视频是输入理解 | — | — | ASR/TTS **也走** `/chat/completions`，使用模型专属 JSON/content 与返回格式，不是 `/audio/*` | — |
| MiniMax | `/chat/completions`（兼容）或文本原生接口；JSON/SSE | `POST /image_generation`；JSON；同步返回 URL | `POST /video_generation` → `GET /query/video_generation?task_id=...` → `/files/retrieve`；异步三段式 | `POST /t2a_v2` 同步/流式；长文本 TTS 为异步任务 | 无仓库通用入口。官方：[API overview](https://platform.minimaxi.com/docs/api-reference/api-overview)、[Video](https://platform.minimaxi.com/docs/guides/video-generation)。 |
| Novita | `/openai/v1/chat/completions`；VLM 图片输入 | v3 模型专用图像接口；部分为异步 `task_id` | `/v3/async/txt2video` 等当前模型 API + task result；异步 | v3 音频模型专用 | OpenAI 兼容 embedding 仅限目录声明的模型。 |
| OpenRouter | `/chat/completions` 或 `/responses`；通过 output modalities 请求媒体的模型仍需能力过滤 | dedicated `/images/generations`；编辑/返回字段按 OpenRouter 文档 | `POST /videos` → `polling_url`；异步 | `/audio/speech`、`/audio/transcriptions`；仅列出的模型 | `/embeddings`；无统一 rerank。官方：[Images](https://openrouter.ai/docs/guides/overview/multimodal/image-generation)、[Video](https://openrouter.ai/docs/guides/overview/multimodal/video-generation)、[Embeddings](https://openrouter.ai/docs/api/reference/embeddings)。 |
| DashScope | 兼容 `/chat/completions` 或原生 multimodal-generation；JSON/SSE | Wan 2.7 可走原生 `/services/aigc/multimodal-generation/generation`（同步）或 `/services/aigc/image-generation/generation` + `X-DashScope-Async: enable`；旧文生图 `/services/aigc/text2image/image-synthesis`；编辑 `/services/aigc/image2image/image-synthesis` | `/services/aigc/video-generation/video-synthesis` + async header，`GET /api/v1/tasks/{id}` | 多套 HTTP/WS 原生 endpoint；批量 ASR 常为 submit/poll，实时为 WS | 原生 text embedding / rerank；部分 embedding 有兼容接口。官方：[Base URL](https://help.aliyun.com/en/model-studio/base-url)。 |
| SiliconFlow | `/chat/completions`；JSON/SSE，VLM 图片输入 | `/images/generations`，但参数与响应是 SiliconFlow 词表；编辑由同路径和 `image` 字段区分 | `/video/submit` + `POST /video/status`；异步 | `/audio/speech` 使用原生 JSON、供应商音色 ID 并返回二进制音频；`/audio/transcriptions` 为 multipart；两者均已路由 ✅ | `/embeddings`；`/rerank`（文本/图像/视频文档），同步。官方：[Create speech](https://docs.siliconflow.com/en/api-reference/audio/create-speech)、[Rerank](https://api-docs.siliconflow.cn/docs/api/rerank-post)。 |
| Zhipu / GLM | `/chat/completions`；JSON/SSE，GLM-V 为输入理解；`glm-4-voice` 也在 Chat 内容块传 `input_audio` 并从 `message.audio` 取结果 | 同步 `/images/generations`；`glm-image` 另有 `/async/images/generations` + `/async-result/{id}`；当前无官方 `/images/edits`。这些是官网能力，仓库当前没有安全匹配其 schema 的图像 serializer | `/videos/generations` 提交，`/async-result/{id}` 查询；CogVideo 与各 Vidu 模型 body 不同 | ASR `/audio/transcriptions` multipart；TTS `/audio/speech` JSON/二进制或 SSE；音色复刻 `/voice/clone`；Realtime `wss://open.bigmodel.cn/api/paas/v4/realtime`。ASR/TTS/Realtime 当前均只记为 `known` | `/embeddings`；`/rerank`；同步。官方：[Image API](https://docs.z.ai/api-reference/image/generate-image)、[HTTP introduction](https://docs.bigmodel.cn/cn/guide/develop/http/introduction)。 |
| Moonshot / Lingyi / ModelScope | OpenAI-compatible `/chat/completions`；部分模型支持图片输入 | — | — | — | 只有供应商目录明确列出且仓库有适配时才开放；不能由 OpenAI base 推导。 |
| xAI | `/chat/completions` 或 `/responses`；JSON/SSE | 生成 `/images/generations`；编辑 `/images/edits`，body 为 JSON `image/images`（URL、base64 或 `file_id`），不是 multipart | 生成 `/videos/generations`、编辑 `/videos/edits`、扩展 `/videos/extensions`，再 `GET /videos/{request_id}`；异步 | `POST /tts`（JSON，音频 base64/流）与 `POST /stt`（multipart）；实时分别使用 `wss://api.x.ai/v1/tts`、`/stt`、`/realtime` | 无通用 embedding/rerank。官方：[Imagine files/input](https://docs.x.ai/developers/model-capabilities/imagine/files/inputs)、[Video](https://docs.x.ai/developers/model-capabilities/video/generation)、[Voice](https://docs.x.ai/developers/model-capabilities/audio/voice)。 |
| Ark / 火山语音 | Ark `/chat/completions` / Responses；Bearer | Ark `/images/generations`，模型参数白名单 | Ark 内容生成 task submit/query；异步 | `openspeech` 独立域名；TTS v1/v3、ASR submit/query/WS 的路径、header 和 client request id 各不相同 | 仅在 Ark 官方端点与模型明确支持时接入；当前不推导。 |
| Qianfan | `/v2/chat/completions`；JSON/SSE | `/v2/images/generations`；编辑 `/v2/images/edits` 是 **JSON**，不是 OpenAI multipart | `POST /video/generations` + `GET /video/generations/{id}`；异步，注意不在 `/v2` 根下 | 百度语音产品使用独立 endpoint、鉴权和任务协议 | `/v2/embeddings`；`/v2/rerank`。官方：[API overview](https://cloud.baidu.com/doc/qianfan-api/s/Dmba8k71y)。 |
| Hunyuan / TokenHub | `/v1/chat/completions`；`hy3*` 还支持 `/v1/responses` 与 `/v1/messages`，其他模型按官方协议矩阵；视觉/视频理解仍输出文本 | `hy-image-lite` 同步 `/v1/api/image/lite`；`hy-image-v3.0` 异步 `/v1/api/image/submit` + `/query`；官方另有 `/v1/images/generations`，新 `hy-image-v3` endpoint 在文档交叉更新期，未确认前不得猜 | `hy-video-1.5`：`POST /v1/api/video/submit` + `/query`；异步。3D 另有 `/v1/api/3d/submit` 与 `/v1/api/3d/query`，仓库尚无 3D task | ASR/TTS 当前通用模型管理所需公开 endpoint 未确认，禁止猜成 `/audio/*` | 文本 `/v1/embeddings`；多模态 `/v1/embeddings/multimodal`，body 不同。官方：[language protocols](https://cloud.tencent.com/document/product/1823/130079)、[video](https://cloud.tencent.com/document/product/1823/130081)、[embeddings](https://cloud.tencent.com/document/product/1823/133515)。 |
| Ctyun / InfiniAI | OpenAI-compatible `/chat/completions`，但必须使用各自当前根和目录 | Ctyun `https://ai.ctaigw.cn/v1/images/generations`；InfiniAI 只按实时目录与官方 endpoint 开放 | — | Ctyun 星辰 MaaS 另有 `/aipaas/voice/v1/tts/supernaturalrt` 与 `/aipaas/voice/v1/asr/fy` WS 协议，只记为 `known`、当前 🧩；不能拼到 TokenHub 根。InfiniAI 只按官网开放 | Ctyun `/embeddings`、`/rerank`；InfiniAI 依实时目录和原生文档。官方：[Ctyun Image API](https://www.ctyun.cn/document/11061839/11062322)、[WS TTS](https://www.ctyun.cn/document/11092117/11093798)、[WS ASR](https://www.ctyun.cn/document/11092117/11093804)。 |
| Poe | `/chat/completions` / `/responses`，bot 协议，能力来自 `/models` metadata | 只对图像 bot 开放专用生成；返回 bot 产物 | `POST /videos` + 查询；异步 | 只对相应 bot/endpoint 开放 | 仅目录明确声明时开放。官方：[List models](https://creator.poe.com/api-reference/listModels)、[Create video](https://creator.poe.com/api-reference/createVideo)。 |
| PPIO | `/openai/v1/chat/completions` | `/v3` 下当前模型专用 endpoint；旧通用 endpoint 已退役 | `/v3` 下当前模型专用 submit/poll；旧通用 txt2video/img2video 已退役 | 按当前模型专用 API | `/openai/v1/embeddings`；rerank 需当前专用文档。 |
| Ctyun | TokenHub `/v1/chat/completions`，含图像理解 | TokenHub `/v1/images/generations` | — | 星辰 MaaS TTS/ASR 是独立 WS path 与鉴权，官网 `known`、当前未适配 🧩 | TokenHub `/v1/embeddings`、`/v1/rerank`；均为同步 JSON。 |
| StepFun | `/chat/completions`；Messages `/v1/messages`；Responses 当前仅 `step-3.7-flash`；双向语音走独立 `wss://api.stepfun.com/v1/realtime`，后端 session/健康探测 ✅、用户产品桥 🧩 | 生成 `/images/generations` JSON；编辑 `/images/edits` multipart（单输入图），两者已使用 `stepfun.images` ✅；图生图 `/images/image2image` JSON 尚未实现，`style_reference` 等未覆盖高级字段也仍 🧩 | 官网当前无视频生成；`video_url` 是 Chat 输入理解 | `/audio/speech`（二进制、URL 或 SSE）已用 `stepfun.audio_speech` ✅；推荐 `stepaudio-2.5-asr` 的 `/audio/asr/sse` JSON+SSE 已用 `stepfun.asr_sse` ✅；文件 ASR、独立流式 ASR WS 与 `/realtime/audio` 流式 TTS WS 仍 🧩 | 仅官网目录列出时接入。官方：[Generate](https://platform.stepfun.com/docs/zh/api-reference/images/image)、[Edit](https://platform.stepfun.com/docs/zh/api-reference/images/edits)、[Image-to-image](https://platform.stepfun.com/docs/zh/api-reference/images/image2image)、[HTTP TTS](https://platform.stepfun.com/docs/zh/api-reference/audio/create-audio)、[WS 流式 TTS](https://platform.stepfun.com/docs/zh/api-reference/audio/ws-audio)、[SSE ASR](https://platform.stepfun.com/docs/zh/api-reference/audio/asr-sse)、[Realtime](https://platform.stepfun.com/docs/zh/api-reference/realtime/chat)、[File ASR](https://platform.stepfun.com/docs/zh/api-reference/audio/asr)、[Streaming ASR](https://platform.stepfun.com/docs/zh/api-reference/audio/asr-stream)。 |
| Step Plan | `/step_plan/v1/chat/completions` 或 `/messages`；`step-router-v1` 仅文本 Chat；双向语音 `/step_plan/v1/realtime` 使用 `stepfun.realtime_s2s` 持久 WS，后端 session/健康探测 ✅、用户产品桥 🧩 | `/step_plan/v1/images/generations`；编辑 `/step_plan/v1/images/edits` multipart；两者使用 `stepfun.images` ✅，未实现边界同普通版 | — | HTTP TTS `/step_plan/v1/audio/speech` 使用 `stepfun.audio_speech` ✅；ASR **仅** `/step_plan/v1/audio/asr/sse` JSON+SSE，使用 `stepfun.asr_sse` ✅；独立流式 TTS WS `/realtime/audio` 尚未实现 🧩，且不等于双向对话任务 | —。官方：[reasoning](https://platform.stepfun.com/docs/zh/step-plan/integrations/reasoning-api)、[image](https://platform.stepfun.com/docs/zh/step-plan/integrations/image-api)、[audio](https://platform.stepfun.com/docs/zh/step-plan/integrations/audio-api)、[Realtime](https://platform.stepfun.com/docs/zh/api-reference/realtime/chat)。 |

## 5. 已确认的退役、弃用与迁移门槛

这里只列本次官网能够确认、且会直接影响仓库现有预置或 fallback 的项目；未列出不代表永不下线。

| 供应商 | 已退役 / 已弃用 / 已公告下线 | 新建配置处理 |
|---|---|---|
| OpenAI | 模型与 snapshot 变化频繁；以官方 [model catalog](https://developers.openai.com/api/docs/models/all) 和各模型页 lifecycle 为准 | 只用动态目录与当前能力页，不维护无截止日期的“全量内置模型”。 |
| Anthropic | 仓库旧 fallback `claude-3-opus-20240229`、`claude-3-sonnet-20240229`、`claude-3-haiku-20240307` 已不能作为当前默认 | 从新增列表移除；旧配置显示 retired 并给替代建议。 |
| AWS Bedrock | 仓库旧默认 `anthropic.claude-sonnet-4-20250514-v1:0` 已为 Legacy；Bedrock 还受区域与 profile lifecycle 影响 | 只展示所在 region 的 online base model/profile；Legacy 不作为默认。 |
| Gemini | Gemini 2.0 于 2026-06-01 shutdown；Veo 多个 `*-preview` endpoint 已迁移/停用 | Developer API 与 Vertex 分别拉目录/生命周期；preview 不做长期 fallback。 |
| DeepSeek | `deepseek-chat`、`deepseek-reasoner` 于 2026-07-24 完全退役 | 新增只展示 `deepseek-v4-flash` / `deepseek-v4-pro`；Responses 暂只给 Flash。 |
| Deepgram | 公共 `/v1/models` 默认返回各模型的最新版本，`include_outdated=true` 只用于历史查询；私有模型属于 project-models 范围 | 不维护永久静态 fallback；从 `stt[]` / `tts[]` 的来源数组保留任务，私有或刚上线的 canonical ID 允许用户手工填写。 |
| MiMo | `mimo-v2-pro`、`mimo-v2-omni`、`mimo-v2-flash`、`mimo-v2-tts` 于 2026-06-30 退役 | 只展示 v2.5 六模型目录；`mimo-v2.5-pro-ultraspeed` 仅有 entitlement 时显示。 |
| MiniMax | `MiniMax-Text-01` 与 abab6.5 一族不再作为当前模型 fallback | 当前文本计划白名单为 M3 / M2.7 系列；媒体按各模态目录。 |
| DashScope | `gte-rerank` 于 2026-05-30 下线；部分旧 Qwen TTS 等已公告 2026-10-10 下线 | 目录保存 `sunset_at`；到期模型不再新增，旧配置倒计时。 |
| Zhipu | GLM-Z1 系列 2025-11-15 弃用；`GLM-4-0520` 2025-12-30 弃用；`glm-4.5-flash` 已公告 2026-01-30 下线并迁往 `glm-4.7-flash` | 即使旧 OpenAPI 枚举残留，也不加入新增列表；记录 `docs_mismatch`。 |
| GLM Coding Plan | `glm-5.1` / `glm-5` 历史调用会自动切到 `glm-5.2` | 仅作为旧配置别名；新增主选 `glm-5.2`、`glm-5-turbo`、`glm-4.7`。 |
| Moonshot | K2 旧 preview 一族于 2026-05-25 下线；`kimi-k2.5` 与所有 `moonshot-v1*` 于 2026-08-31 下线；`kimi-latest` 已于 2026-01-28 下线 | 新增只展示实时目录中的 K3/K2.7/K2.6；将 8 月 31 日模型标 `pre-offline`。 |
| Hunyuan | 大批旧混元文本 ID 已于 2026-06-22 下线；`hy3-preview` 2026-08-31 下线；旧图像 2026-09-15 停止任务；整个旧平台 2026-09-30 停服 | 本轮已把新建默认迁到中国/Global TokenHub profile；旧平台仅保留 Legacy 迁移语义。 |
| PPIO | 通用 v3 文/图生视频入口 2025-07-31 退役；旧通用图像入口 2026-01-31 退役 | 只展示当前模型专用媒体 endpoint；精确命中旧官方预置根的已有配置迁到 `api.ppio.com/openai/v1`。 |
| Ctyun | `wishub-x1` 所属文档已停止维护；`wishub-x6` 也已被现行接口表替换 | 新增只用 `ai.ctaigw.cn/v1`；精确命中旧 x6 官方预置根的已有配置由一次性迁移 032 更新，自定义网关不改。 |
| StepFun | `/audio/transcriptions` 及固定模型 `step-asr` 已进入逐步废弃路径；`step-asr-1.1-stream` 只保留旧流式兼容；`step-tts-vivid` 不再推荐 | 普通版以实时 `/models` 为准；新建 ASR 优先 `stepaudio-2.5-asr` + `stepfun.asr_sse`，TTS 优先当前 `stepaudio-2.5-tts` / 官网目录。 |

## 6. 发布与维护门槛

每次新增供应商或模态必须同时提交：

- 官方根地址、地区/计划/key 边界，以及官网来源 URL；
- 模型目录方式与空目录/权限失败策略；
- `(platform, task, model)` endpoint、HTTP/WS、鉴权、Content-Type、同步/异步、轮询和结果物化规则；
- 至少一组请求/响应契约测试，确认没有重复 `/v1`、没有把兼容根拼进原生根；
- 当前模型或计划 allowlist、`verified_at`、deprecated/retired 迁移测试；
- UI 的“供应商 → 主类型 → 统一模型输入 → 额外任务”顺序、自由模型 ID 输入和目录失败不阻塞保存测试；目录选择只应用主类型对应的已核验 traits，不得自动加入目录声明的其他任务；手填模型先单选主类型，再通过“添加其他任务”逐项扩展；还需覆盖“已适配 / 官网已知待适配 / 自定义需覆盖”三态，且供应商 adapter 状态不得被当作模型能力，只有 `supported` 能自动命中预置路由。

上线前必须检查两类高风险回归：

1. 已知供应商的未知任务必须失败关闭，不能命中 generic OpenAI fallback；
2. 旧配置如果保存了已退役模型，允许显示“已退役/不可用”并引导迁移，但不能把它重新放回新建模型下拉列表。

本轮数据库迁移只对 `platform='custom'` 且 base URL **精确等于**旧版官方预置根的记录恢复供应商身份；真正的自定义网关和 `new-api` 即使碰巧代理同一域名也不会被改写。PPIO 与天翼的已停止维护根会同时改到当前根，其他供应商只恢复 `platform`，不擅自修改用户 URL。

本文是 2026-08-11 的核验快照，不是永久模型清单。动态目录和供应商生命周期公告始终高于本文中的示例模型；静态计划白名单应在每次发布前重新核验。
