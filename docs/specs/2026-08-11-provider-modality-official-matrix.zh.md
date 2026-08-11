# 模型供应商 × 模态官方接口核验矩阵（2026-08-11）

> 核验日期：2026-08-11（Asia/Shanghai）。范围是 `MODEL_PLATFORMS` 中的全部预置项，以及 `custom` / `new-api` 两个用户定义入口。本文只记录供应商官网公开接口；模型是否对某一账号、地区或订阅开放，仍以该账号实时目录和控制台为准。

## 1. 结论与强制规则

“OpenAI 兼容”通常只表示某一个入口（多为 Chat Completions）兼容，并不表示图像、视频、语音、向量和重排也兼容。仓库不得再用一个统一 `base_url` 和一个统一协议推导所有模态。

配置顺序固定为：

1. 选择供应商（含地区、计费方式或订阅计划）；
2. 选择模态；
3. 只展示该供应商、该模态当前可调用的模型；
4. 根据 `(platform, task, model)` 选择 endpoint、鉴权、请求/响应序列化器和同步/异步状态机；
5. 没有原生适配器时返回明确的 `UnsupportedTask` / `NoAdapter`，绝不回退到 OpenAI 路径“试一下”。

模型目录策略也必须分开：

- 有官方运行时目录的供应商以实时目录为准，并缓存 `verified_at`、来源 URL 和能力元数据；空目录、401/403、地区无权限均不能用陈旧硬编码列表伪装成功。
- 没有目录 API 的订阅计划才使用官网白名单；白名单必须带核验日期和来源，过期后应阻止发布，而不是永久保留模型。
- 目录返回模型名不等于该模型支持所有任务。任务要来自官方能力字段、不同模态目录或经过核验的模型族规则。
- `vision` 指图像/音频/视频**输入理解**，仍属于 Chat；它不能自动获得图像/音频/视频**生成**任务。

本文状态符号：`✅` 已有本仓库原生路由；`⛔` 本轮主动隐藏/拒绝，防止误走 OpenAI；`🧩` 官网支持但仍待原生适配；`—` 官网未提供或不适用。状态描述的是本仓库，不是供应商能力上限。

## 2. 供应商、地区、计划与目录总表

| `platform` / 预置项 | 官方根地址与鉴权边界 | 当前目录策略 | 生命周期与仓库策略 |
|---|---|---|---|
| `custom` | 用户给定；鉴权、协议和完整 endpoint 都是用户数据 | 不猜测；可手工模型，或显式配置目录 URL/解析器 | 仅作为高级逃生舱。用户必须逐模型选择协议与任务，不能因名称像某供应商而自动改协议。 |
| `new-api` | 用户网关根；每个部署启用的上游不同 | `GET {base}/v1/models`，但仅能证明网关公开了 ID | 每模型显式 `openai` / `anthropic` / `gemini`；非 Chat 模态只有网关声明且仓库有对应协议适配器时才展示。 |
| `openai` | `https://api.openai.com/v1`，Bearer | 动态 `GET /models`，再按官方模型页/endpoint 能力分组 | 不保存“永久可用”静态列表；下架与弃用以 [Models](https://developers.openai.com/api/docs/models/all) 和官方弃用信息为准。 |
| `anthropic` | `https://api.anthropic.com`；`x-api-key` + `anthropic-version` | 动态 `GET /v1/models` | 仓库旧 fallback 中 `claude-3-opus-20240229`、`claude-3-sonnet-20240229`、`claude-3-haiku-20240307` 已不可继续作为当前默认；按 [Models](https://docs.anthropic.com/en/docs/about-claude/models/overview) 与 [deprecations](https://docs.anthropic.com/en/docs/resources/model-deprecations) 更新。仅 Chat ✅。 |
| `bedrock` | 控制面 `https://bedrock.{region}.amazonaws.com`、Native `bedrock-runtime`、OpenAI/Anthropic 兼容面 `bedrock-mantle`；AWS SigV4，不是 API Key base | `ListFoundationModels` + `ListInferenceProfiles` + Mantle `/v1/models`；模型 ID、profile、区域和账号权限共同决定可用性 | 同一模型可能只支持 Converse、原生 Invoke、Mantle、AsyncInvoke、Sonic 双向流或 agent-runtime rerank 中一部分。仓库当前 Chat provider 仅实现 Claude `invoke-with-response-stream`，统一 invoke 层仍不完整，且旧默认 Sonnet 4 已为 Legacy；除已验证 Claude Chat 外均 ⛔/🧩。以 [model availability](https://docs.aws.amazon.com/bedrock/latest/userguide/models.html) 为准。 |
| `gemini` | `https://generativelanguage.googleapis.com`；`x-goog-api-key`，稳定接口用 `v1`、预览接口用 `v1beta` | 动态 `GET /v1beta/models`，读取 `supportedGenerationMethods` | 当前 Chat 主线为 Gemini 3.6/3.5/3.1；Gemini 2.0 已于 2026-06-01 shutdown。`*-preview`、实验 ID 和别名按官方 [models](https://ai.google.dev/gemini-api/docs/models) / deprecation 表更新，不能作为永久 fallback。Chat、图像生成/编辑 ✅；其他官网能力 🧩。 |
| `gemini-vertex-ai` | `https://{location}-aiplatform.googleapis.com/v1/projects/{project}/locations/{location}`；OAuth2/ADC | Google publisher model + 项目/区域可用性；不能复用 Gemini Developer API 的 key 与目录 | 必须拆成 `publishers/google` 的 Gemini 与 `publishers/anthropic` 的 Claude。旧预置会把 UI 的 Gemini 2.5 发往 Anthropic `streamRawPredict`，属于确定性错配；本轮已将它从“新建供应商”列表移除 ⛔，待拆分成两个正确产品面后再恢复。预览 Veo endpoint 迁移见 [Vertex release notes](https://cloud.google.com/vertex-ai/docs/release-notes)。 |
| `deepseek` | `https://api.deepseek.com`（`/v1` 兼容别名可用），Bearer | 官方当前目录/更新页；无目录时只允许核验白名单 | 2026-07-24 后 `deepseek-chat`、`deepseek-reasoner` 已退役；当前为 `deepseek-v4-flash`、`deepseek-v4-pro`。仅文本 Chat/Responses；见 [updates](https://api-docs.deepseek.com/updates)。Chat ✅。 |
| `mimo` | `https://api.xiaomimimo.com/v1`；`api-key` 或 Bearer | 动态 `GET /models`；截至核验日精确返回 6 个 v2.5 ID | 当前：`mimo-v2.5-pro`、`mimo-v2.5`、`mimo-v2.5-asr`、`mimo-v2.5-tts`、`mimo-v2.5-tts-voicedesign`、`mimo-v2.5-tts-voiceclone`，见 [official list](https://mimo.mi.com/docs/en-US/api/model/list-models)。旧 `mimo-v2-pro`、`mimo-v2-omni`、`mimo-v2-flash`、旧 `mimo-v2-tts` 于 2026-06-30 退役；本轮 Chat、ASR、TTS 均已按 `/chat/completions` 的模型专属序列化原生适配 ✅。 |
| `mimo-token-plan-cn` / `sgp` / `ams` | `https://token-plan-cn.xiaomimimo.com/v1`、`https://token-plan-sgp.xiaomimimo.com/v1`、`https://token-plan-ams.xiaomimimo.com/v1`；`tp-` key | 计划白名单/计划端目录，按区域分别缓存 | 计划 key 与按量 `sk-` key 不互通，且计划仅授权官方 coding tools，不应作为通用后端推理入口；仅 Chat ✅。 |
| `minimax` | 中国 `https://api.minimaxi.com/v1`；Bearer | 官网跨模态模型页；没有单一、可依赖的全模态目录时使用按模态核验白名单 | 当前文本主线含 `MiniMax-M3`、`MiniMax-M2.7`、`MiniMax-M2.7-highspeed`；旧 `MiniMax-Text-01`、abab 别名不得 fallback。当前接口不再强制 `GroupId`，仓库不能因缺少 GroupId 拒绝。Chat/TTS ✅；图像/视频 🧩。 |
| `minimax-code` | 国际 `https://api.minimax.io/v1`；国际 key | 与中国站分别维护官网白名单 | 中国/国际 key 不互通；当前仓库仅 Chat ✅。 |
| `minimax-coding-plan` | 中国计划 gateway；计划 key | 静态计划 allowlist：`MiniMax-M3`、`MiniMax-M2.7`、`MiniMax-M2.7-highspeed` | 计划只展示 Chat；其他模态 ⛔。来源见 [MiniMax API overview](https://platform.minimaxi.com/docs/api-reference/api-overview)。 |
| `novita` | LLM：`https://api.novita.ai/openai/v1`；媒体：`https://api.novita.ai/v3`，Bearer | LLM `/models` 与媒体 `/v3/model`/各产品模型目录分开 | “OpenAI”根不能调用 v3 图像/视频任务。Chat/Embedding ✅；图像、视频、音频官网可用但 🧩。见 [Novita model API](https://novita.ai/docs/api-reference/model-apis-get-model)。 |
| `openrouter` | `https://openrouter.ai/api/v1`，Bearer | 动态 `/models`；图像还可用 `/images/models`，必须读 `architecture.input_modalities` / `output_modalities` | 不能把所有聚合模型都标成所有模态。Chat/Embedding ✅；图像/视频/音频 ⛔/🧩。见 [multimodal overview](https://openrouter.ai/docs/guides/overview/multimodal/overview)。 |
| `dashscope` | 北京：兼容 `https://dashscope.aliyuncs.com/compatible-mode/v1`，原生 `https://dashscope.aliyuncs.com/api/v1`，WS `wss://dashscope.aliyuncs.com/api-ws/v1/inference`；新加坡/美国用 `-intl` / `-us` 独立域名 | 兼容 `/models` + [官方模型目录](https://help.aliyun.com/en/model-studio/models)，按地域过滤 | 兼容根与原生根不能拼接；`gte-rerank` 已于 2026-05-30 下线，更多模型按 [deprecation](https://help.aliyun.com/en/model-studio/model-depreciation) 处理。Chat、图像生成、Embedding ✅；其余 🧩。 |
| `dashscope-coding` | OpenAI：`https://coding.dashscope.aliyuncs.com/v1`；Anthropic：`https://coding.dashscope.aliyuncs.com/apps/anthropic` | 官网静态 allowlist | 当前精确白名单：`qwen3.7-plus`、`qwen3.6-plus`、`kimi-k2.5`、`glm-5`、`MiniMax-M2.5`、`qwen3.5-plus`、`qwen3-max-2026-01-23`、`qwen3-coder-next`、`qwen3-coder-plus`、`glm-4.7`；见 [Coding Plan](https://help.aliyun.com/en/model-studio/coding-plan)。仅 Chat ✅。 |
| `siliconflow`（CN / Global） | `https://api.siliconflow.cn/v1` / `https://api.siliconflow.com/v1`，Bearer；账号与模型可用区分站 | 动态 `/models`，按模型 `type`/endpoint 目录分模态 | 图像响应、视频 submit/status、音频字段均非“只换 base”的 OpenAI 等价物。本轮 Chat、图像生成/编辑、视频 submit/status、ASR、Embedding 与同步 Rerank 均已适配 ✅；TTS 仍 ⛔/🧩。见 [API docs](https://docs.siliconflow.com/en/api-reference/)。 |
| `zhipu` | 中国 `https://open.bigmodel.cn/api/paas/v4`，Bearer | 官方 OpenAPI 没有公共 `GET /models`，因此使用按模态、带来源和日期的维护目录 | 当前目录覆盖文本、视觉理解、`glm-4-voice`、`glm-image`/CogView、已核准精确 ID 的 CogVideo、`glm-asr-2512`、`glm-tts`、`embedding-2/3`、`rerank`；Vidu 虽共用视频协议，但在精确 callable ID 未全部进入总览前不猜别名。本轮 Chat、图像、视频异步任务、ASR、Embedding、Rerank 已按验证协议适配 ✅；TTS 仍 🧩 且不在 UI 中误开放。见 [模型总览](https://docs.bigmodel.cn/cn/guide/start/model-overview)、[视频生成](https://docs.bigmodel.cn/api-reference/模型-api/视频生成异步) 与 [查询异步结果](https://docs.bigmodel.cn/api-reference/模型-api/查询异步结果)。 |
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
| `ctyun` | `https://wishub-x6.ctyun.cn/v1`，Bearer AppKey | `GET /models`/控制台在线服务目录；按接口类型过滤 | 旧 `wishub-x1` 文档已停止维护。x6 明确提供 Chat/VLM、Image、Embedding、Rerank；本轮四类均已适配 ✅。见 [接口类型列表](https://www.ctyun.cn/document/11061839/11062345)。 |
| `stepfun` | `https://api.stepfun.com/v1`，Bearer；Anthropic SDK base 是 `https://api.stepfun.com`；Realtime 为独立 WS | 动态 `/models`；仅官网主机临时不可用时允许带日期的公开模型 fallback | 本轮 Chat、图像生成/编辑、ASR 已走验证兼容适配 ✅；TTS、Realtime 与文件/流式 ASR 中未注册的协议 ⛔/🧩。`step-asr` 是 legacy，`step-asr-1.1-stream` 逐步废弃，`step-tts-vivid` 已“不再推荐”；见 [TTS](https://platform.stepfun.com/docs/zh/api-reference/audio/create-audio)。 |
| `stepfun-plan` | OpenAI `https://api.stepfun.com/step_plan/v1`；Anthropic SDK 根 `https://api.stepfun.com/step_plan`；Realtime `wss://api.stepfun.com/step_plan/v1/realtime` | 无官方 `/models`；官网当前静态 allowlist 共 9 个：`step-3.7-flash`、`step-3.5-flash`、`step-3.5-flash-2603`、`stepaudio-2.5-realtime`、`stepaudio-2.5-chat`、`stepaudio-2.5-tts`、`stepaudio-2.5-asr`、`step-router-v1`、`step-image-edit-2` | `step-router-v1` 是 Chat 路由，不接受图片/文档；计划本身还支持图像、TTS、ASR、Realtime。本轮 Chat、图像生成/编辑已走验证兼容适配 ✅；TTS/ASR/Realtime 仍 ⛔/🧩。见 [Step Plan overview](https://platform.stepfun.com/docs/zh/step-plan/overview)。 |

## 3. 每模态 endpoint / 协议 / 同异步矩阵

下表列出供应商公开能力。`输入` 表示将图片/音频/视频放入 Chat 内容；不产生媒体文件。省略的 endpoint 不是“沿用 OpenAI”，而是该预置当前不应暴露该任务。

| 供应商族 | Chat / 多模态理解 | 图像生成 / 编辑 | 视频生成 | TTS / ASR | Embedding / Rerank |
|---|---|---|---|---|---|
| OpenAI | `POST /responses` 或 `/chat/completions`；JSON；同步或 SSE | `POST /images/generations`；编辑 `POST /images/edits` multipart；同步任务响应 | `POST /videos`，`GET /videos/{id}` 轮询，`GET /videos/{id}/content`；异步 | `/audio/speech` 返回音频流；`/audio/transcriptions` multipart；同步/流式依模型 | `/embeddings` 同步；无通用 rerank。官方：[Images](https://developers.openai.com/api/docs/guides/image-generation)、[Video](https://developers.openai.com/api/docs/guides/video-generation)、[Audio](https://developers.openai.com/api/docs/guides/text-to-speech)。 |
| Anthropic | `POST /v1/messages`，JSON，同步/SSE；图片/PDF是输入内容块 | — | — | — | — |
| Bedrock | `Converse/ConverseStream` 或 `POST /model/{modelId}/invoke[...stream]`，SigV4；body 由模型族决定 | 同一 InvokeModel 路径但 body/响应由图像模型决定 | 模型专属异步 invoke/job；不能假设统一 poll | 模型专属 InvokeModel/流；不能复用 OpenAI Audio | embedding 走模型专属 InvokeModel；不是统一 `/embeddings`。官方：[InvokeModel](https://docs.aws.amazon.com/bedrock/latest/APIReference/API_runtime_InvokeModel.html)。 |
| Gemini Developer API | `POST /v1beta/models/{model}:generateContent` 或 `:streamGenerateContent?alt=sse`；内容为 `contents.parts` | Gemini image-output 走 `generateContent`；Imagen 使用专用预测协议；编辑仍是内容 parts，不是 OpenAI multipart | Veo long-running operation，提交后按 operation name 轮询 | TTS 可由 `generateContent` 请求 audio 输出；Live/实时走 WebSocket；ASR 多为理解输入 | `models/{model}:embedContent` / `batchEmbedContents`；无通用 rerank。官方：[API reference](https://ai.google.dev/api)。 |
| Vertex AI | `.../publishers/google/models/{model}:generateContent` / `:streamGenerateContent`，OAuth | Imagen `:predict`，请求为 `instances/parameters` | Veo `:predictLongRunning` + operation 轮询 | Live API/模型专用 endpoint | `:predict` 或 `:embedContent`（依模型）；均受 project/location 限制。 |
| DeepSeek | `/chat/completions`；另有 `/responses`（Flash）和 `/beta/completions` FIM（Pro）；JSON/SSE | — | — | — | — |
| MiMo | `/chat/completions`；`mimo-v2.5` 的图片/音频/视频是输入理解 | — | — | ASR/TTS **也走** `/chat/completions`，使用模型专属 JSON/content 与返回格式，不是 `/audio/*` | — |
| MiniMax | `/chat/completions`（兼容）或文本原生接口；JSON/SSE | `POST /image_generation`；JSON；同步返回 URL | `POST /video_generation` → `GET /query/video_generation?task_id=...` → `/files/retrieve`；异步三段式 | `POST /t2a_v2` 同步/流式；长文本 TTS 为异步任务 | 无仓库通用入口。官方：[API overview](https://platform.minimaxi.com/docs/api-reference/api-overview)、[Video](https://platform.minimaxi.com/docs/guides/video-generation)。 |
| Novita | `/openai/v1/chat/completions`；VLM 图片输入 | v3 模型专用图像接口；部分为异步 `task_id` | `/v3/async/txt2video` 等当前模型 API + task result；异步 | v3 音频模型专用 | OpenAI 兼容 embedding 仅限目录声明的模型。 |
| OpenRouter | `/chat/completions` 或 `/responses`；通过 output modalities 请求媒体的模型仍需能力过滤 | dedicated `/images/generations`；编辑/返回字段按 OpenRouter 文档 | `POST /videos` → `polling_url`；异步 | `/audio/speech`、`/audio/transcriptions`；仅列出的模型 | `/embeddings`；无统一 rerank。官方：[Images](https://openrouter.ai/docs/guides/overview/multimodal/image-generation)、[Video](https://openrouter.ai/docs/guides/overview/multimodal/video-generation)、[Embeddings](https://openrouter.ai/docs/api/reference/embeddings)。 |
| DashScope | 兼容 `/chat/completions` 或原生 multimodal-generation；JSON/SSE | Wan 2.7 可走原生 `/services/aigc/multimodal-generation/generation`（同步）或 `/services/aigc/image-generation/generation` + `X-DashScope-Async: enable`；旧文生图 `/services/aigc/text2image/image-synthesis`；编辑 `/services/aigc/image2image/image-synthesis` | `/services/aigc/video-generation/video-synthesis` + async header，`GET /api/v1/tasks/{id}` | 多套 HTTP/WS 原生 endpoint；批量 ASR 常为 submit/poll，实时为 WS | 原生 text embedding / rerank；部分 embedding 有兼容接口。官方：[Base URL](https://help.aliyun.com/en/model-studio/base-url)。 |
| SiliconFlow | `/chat/completions`；JSON/SSE，VLM 图片输入 | `/images/generations`，但参数与响应是 SiliconFlow 词表；编辑由同路径和 `image` 字段区分 | `/video/submit` + `POST /video/status`；异步 | `/audio/speech`、`/audio/transcriptions`，字段/返回需原生校验 | `/embeddings`；`/rerank`（文本/图像/视频文档），同步。官方：[Rerank](https://api-docs.siliconflow.cn/docs/api/rerank-post)。 |
| Zhipu / GLM | `/chat/completions`；JSON/SSE，GLM-V 为输入理解；`glm-4-voice` 也在 Chat 内容块传 `input_audio` 并从 `message.audio` 取结果 | 同步 `/images/generations`；`glm-image` 另有 `/async/images/generations` + `/async-result/{id}`；当前无官方 `/images/edits` | `/videos/generations` 提交，`/async-result/{id}` 查询；CogVideo 与各 Vidu 模型 body 不同 | ASR `/audio/transcriptions` multipart；TTS `/audio/speech` JSON/二进制或 SSE；音色复刻 `/voice/clone`；Realtime `wss://open.bigmodel.cn/api/paas/v4/realtime` | `/embeddings`；`/rerank`；同步。官方：[Image API](https://docs.z.ai/api-reference/image/generate-image)、[HTTP introduction](https://docs.bigmodel.cn/cn/guide/develop/http/introduction)。 |
| Moonshot / Lingyi / ModelScope | OpenAI-compatible `/chat/completions`；部分模型支持图片输入 | — | — | — | 只有供应商目录明确列出且仓库有适配时才开放；不能由 OpenAI base 推导。 |
| xAI | `/chat/completions` 或 `/responses`；JSON/SSE | 生成 `/images/generations`；编辑 `/images/edits`，body 为 JSON `image/images`（URL、base64 或 `file_id`），不是 multipart | 生成 `/videos/generations`、编辑 `/videos/edits`、扩展 `/videos/extensions`，再 `GET /videos/{request_id}`；异步 | `POST /tts`（JSON，音频 base64/流）与 `POST /stt`（multipart）；实时分别使用 `wss://api.x.ai/v1/tts`、`/stt`、`/realtime` | 无通用 embedding/rerank。官方：[Imagine files/input](https://docs.x.ai/developers/model-capabilities/imagine/files/inputs)、[Video](https://docs.x.ai/developers/model-capabilities/video/generation)、[Voice](https://docs.x.ai/developers/model-capabilities/audio/voice)。 |
| Ark / 火山语音 | Ark `/chat/completions` / Responses；Bearer | Ark `/images/generations`，模型参数白名单 | Ark 内容生成 task submit/query；异步 | `openspeech` 独立域名；TTS v1/v3、ASR submit/query/WS 的路径、header 和 client request id 各不相同 | 仅在 Ark 官方端点与模型明确支持时接入；当前不推导。 |
| Qianfan | `/v2/chat/completions`；JSON/SSE | `/v2/images/generations`；编辑 `/v2/images/edits` 是 **JSON**，不是 OpenAI multipart | `POST /video/generations` + `GET /video/generations/{id}`；异步，注意不在 `/v2` 根下 | 百度语音产品使用独立 endpoint、鉴权和任务协议 | `/v2/embeddings`；`/v2/rerank`。官方：[API overview](https://cloud.baidu.com/doc/qianfan-api/s/Dmba8k71y)。 |
| Hunyuan / TokenHub | `/v1/chat/completions`；`hy3*` 还支持 `/v1/responses` 与 `/v1/messages`，其他模型按官方协议矩阵；视觉/视频理解仍输出文本 | `hy-image-lite` 同步 `/v1/api/image/lite`；`hy-image-v3.0` 异步 `/v1/api/image/submit` + `/query`；官方另有 `/v1/images/generations`，新 `hy-image-v3` endpoint 在文档交叉更新期，未确认前不得猜 | `hy-video-1.5`：`POST /v1/api/video/submit` + `/query`；异步。3D 另有 `/v1/api/3d/submit` 与 `/v1/api/3d/query`，仓库尚无 3D task | ASR/TTS 当前通用模型管理所需公开 endpoint 未确认，禁止猜成 `/audio/*` | 文本 `/v1/embeddings`；多模态 `/v1/embeddings/multimodal`，body 不同。官方：[language protocols](https://cloud.tencent.com/document/product/1823/130079)、[video](https://cloud.tencent.com/document/product/1823/130081)、[embeddings](https://cloud.tencent.com/document/product/1823/133515)。 |
| Ctyun / InfiniAI | OpenAI-compatible `/chat/completions`，但必须使用各自当前根和目录 | Ctyun `/images/generations`；InfiniAI 只按实时目录与官方 endpoint 开放 | — | — | Ctyun `/embeddings`、`/rerank`；InfiniAI 依实时目录和原生文档。 |
| Poe | `/chat/completions` / `/responses`，bot 协议，能力来自 `/models` metadata | 只对图像 bot 开放专用生成；返回 bot 产物 | `POST /videos` + 查询；异步 | 只对相应 bot/endpoint 开放 | 仅目录明确声明时开放。官方：[List models](https://creator.poe.com/api-reference/listModels)、[Create video](https://creator.poe.com/api-reference/createVideo)。 |
| PPIO | `/openai/v1/chat/completions` | `/v3` 下当前模型专用 endpoint；旧通用 endpoint 已退役 | `/v3` 下当前模型专用 submit/poll；旧通用 txt2video/img2video 已退役 | 按当前模型专用 API | `/openai/v1/embeddings`；rerank 需当前专用文档。 |
| Ctyun | `/v1/chat/completions`，含图像理解 | `/v1/images/generations` | — | — | `/v1/embeddings`、`/v1/rerank`；均为同步 JSON。 |
| StepFun | `/chat/completions`；Messages `/v1/messages`；Responses 当前仅 `step-3.7-flash`；Realtime `wss://api.stepfun.com/v1/realtime` | 生成 `/images/generations` JSON；编辑 `/images/edits` multipart（单输入图）；图生图 `/images/image2image` JSON，三者不能合并成一个 serializer | 官网当前无视频生成；`video_url` 是 Chat 输入理解 | `/audio/speech`（二进制、URL 或 SSE）；基础 ASR `/audio/transcriptions` multipart；文件 ASR `/audio/asr/file/submit` + `/query`；SSE ASR `/audio/asr/sse`；实时 ASR WS | 仅官网目录列出时接入。官方：[Generate](https://platform.stepfun.com/docs/zh/api-reference/images/image)、[Edit](https://platform.stepfun.com/docs/zh/api-reference/images/edits)、[File ASR](https://platform.stepfun.com/docs/zh/api-reference/audio/asr)、[Streaming ASR](https://platform.stepfun.com/docs/zh/api-reference/audio/asr-stream)。 |
| Step Plan | `/step_plan/v1/chat/completions` 或 `/messages`；`step-router-v1` 仅文本 Chat | `/step_plan/v1/images/generations`；编辑 `/step_plan/v1/images/edits` multipart | — | TTS `/step_plan/v1/audio/speech`，流式 WS `/realtime/audio`；ASR **仅** `/step_plan/v1/audio/asr/sse` JSON+SSE；Realtime `/step_plan/v1/realtime` | —。官方：[reasoning](https://platform.stepfun.com/docs/zh/step-plan/integrations/reasoning-api)、[image](https://platform.stepfun.com/docs/zh/step-plan/integrations/image-api)、[audio](https://platform.stepfun.com/docs/zh/step-plan/integrations/audio-api)。 |

## 4. 已确认的退役、弃用与迁移门槛

这里只列本次官网能够确认、且会直接影响仓库现有预置或 fallback 的项目；未列出不代表永不下线。

| 供应商 | 已退役 / 已弃用 / 已公告下线 | 新建配置处理 |
|---|---|---|
| OpenAI | 模型与 snapshot 变化频繁；以官方 [model catalog](https://developers.openai.com/api/docs/models/all) 和各模型页 lifecycle 为准 | 只用动态目录与当前能力页，不维护无截止日期的“全量内置模型”。 |
| Anthropic | 仓库旧 fallback `claude-3-opus-20240229`、`claude-3-sonnet-20240229`、`claude-3-haiku-20240307` 已不能作为当前默认 | 从新增列表移除；旧配置显示 retired 并给替代建议。 |
| AWS Bedrock | 仓库旧默认 `anthropic.claude-sonnet-4-20250514-v1:0` 已为 Legacy；Bedrock 还受区域与 profile lifecycle 影响 | 只展示所在 region 的 online base model/profile；Legacy 不作为默认。 |
| Gemini | Gemini 2.0 于 2026-06-01 shutdown；Veo 多个 `*-preview` endpoint 已迁移/停用 | Developer API 与 Vertex 分别拉目录/生命周期；preview 不做长期 fallback。 |
| DeepSeek | `deepseek-chat`、`deepseek-reasoner` 于 2026-07-24 完全退役 | 新增只展示 `deepseek-v4-flash` / `deepseek-v4-pro`；Responses 暂只给 Flash。 |
| MiMo | `mimo-v2-pro`、`mimo-v2-omni`、`mimo-v2-flash`、`mimo-v2-tts` 于 2026-06-30 退役 | 只展示 v2.5 六模型目录；`mimo-v2.5-pro-ultraspeed` 仅有 entitlement 时显示。 |
| MiniMax | `MiniMax-Text-01` 与 abab6.5 一族不再作为当前模型 fallback | 当前文本计划白名单为 M3 / M2.7 系列；媒体按各模态目录。 |
| DashScope | `gte-rerank` 于 2026-05-30 下线；部分旧 Qwen TTS 等已公告 2026-10-10 下线 | 目录保存 `sunset_at`；到期模型不再新增，旧配置倒计时。 |
| Zhipu | GLM-Z1 系列 2025-11-15 弃用；`GLM-4-0520` 2025-12-30 弃用；`glm-4.5-flash` 已公告 2026-01-30 下线并迁往 `glm-4.7-flash` | 即使旧 OpenAPI 枚举残留，也不加入新增列表；记录 `docs_mismatch`。 |
| GLM Coding Plan | `glm-5.1` / `glm-5` 历史调用会自动切到 `glm-5.2` | 仅作为旧配置别名；新增主选 `glm-5.2`、`glm-5-turbo`、`glm-4.7`。 |
| Moonshot | K2 旧 preview 一族于 2026-05-25 下线；`kimi-k2.5` 与所有 `moonshot-v1*` 于 2026-08-31 下线；`kimi-latest` 已于 2026-01-28 下线 | 新增只展示实时目录中的 K3/K2.7/K2.6；将 8 月 31 日模型标 `pre-offline`。 |
| Hunyuan | 大批旧混元文本 ID 已于 2026-06-22 下线；`hy3-preview` 2026-08-31 下线；旧图像 2026-09-15 停止任务；整个旧平台 2026-09-30 停服 | 本轮已把新建默认迁到中国/Global TokenHub profile；旧平台仅保留 Legacy 迁移语义。 |
| PPIO | 通用 v3 文/图生视频入口 2025-07-31 退役；旧通用图像入口 2026-01-31 退役 | 只展示当前模型专用媒体 endpoint；精确命中旧官方预置根的已有配置迁到 `api.ppio.com/openai/v1`。 |
| Ctyun | `wishub-x1` 所属文档已停止维护 | 新增只用 `wishub-x6.ctyun.cn/v1`；精确命中旧官方预置根的已有配置一并迁移。 |
| StepFun | `step-asr` 为 legacy；`step-asr-1.1-stream` 逐步废弃；`step-tts-vivid` 不再推荐 | 普通版以实时 `/models` 为准；新配置优先 `stepaudio-2.5-*` / 当前模型。 |

## 5. 发布与维护门槛

每次新增供应商或模态必须同时提交：

- 官方根地址、地区/计划/key 边界，以及官网来源 URL；
- 模型目录方式与空目录/权限失败策略；
- `(platform, task, model)` endpoint、HTTP/WS、鉴权、Content-Type、同步/异步、轮询和结果物化规则；
- 至少一组请求/响应契约测试，确认没有重复 `/v1`、没有把兼容根拼进原生根；
- 当前模型或计划 allowlist、`verified_at`、deprecated/retired 迁移测试；
- UI 的“供应商 → 模态 → 模型”过滤测试，以及“不支持任务不会出现”的测试。

上线前必须检查两类高风险回归：

1. 已知供应商的未知任务必须失败关闭，不能命中 generic OpenAI fallback；
2. 旧配置如果保存了已退役模型，允许显示“已退役/不可用”并引导迁移，但不能把它重新放回新建模型下拉列表。

本轮数据库迁移只对 `platform='custom'` 且 base URL **精确等于**旧版官方预置根的记录恢复供应商身份；真正的自定义网关和 `new-api` 即使碰巧代理同一域名也不会被改写。PPIO 与天翼的已停止维护根会同时改到当前根，其他供应商只恢复 `platform`，不擅自修改用户 URL。

本文是 2026-08-11 的核验快照，不是永久模型清单。动态目录和供应商生命周期公告始终高于本文中的示例模型；静态计划白名单应在每次发布前重新核验。
