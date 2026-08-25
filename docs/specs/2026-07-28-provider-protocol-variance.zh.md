# 多供应商非 Chat 模态 API 协议差异调研报告(2026-07)

## 一、总览差异矩阵

说明:「同 chat 域」指与该厂商 chat/completions 同一基础域名与同一凭证;鉴权若无特别说明均为 `Authorization: Bearer <key>`。标注「※」的条目来自二手资料或训练期知识,建议接入前用一次真实调用核实;「未确认」为无法交叉验证的项。

### 1. OpenAI(api.openai.com,全模态同域同凭证)

| 模态 | 路径 | 同/异步 | 请求形状 | 响应形状 |
|---|---|---|---|---|
| 图像生成 | `POST /v1/images/generations` | 同步(可 stream 部分图) | JSON | dall-e: url 或 b64_json(受 `response_format` 控制);**gpt-image-1: 恒为 b64_json,传 `response_format` 报错** |
| 图像编辑 | `POST /v1/images/edits` | 同步 | **multipart/form-data**,多图用 `image[]` 重复字段(gpt-image-1 至多 16 张),`mask` 为带 alpha 的 PNG | b64_json;`stream=true`+`partial_images`(0-3) 走 SSE 事件 `image_edit.partial_image` |
| TTS | `POST /v1/audio/speech` | 同步 | JSON(`model/input/voice/instructions`) | **裸二进制音频**(mp3/opus/aac/flac/wav/pcm),支持分块流式 |
| ASR | `POST /v1/audio/transcriptions` | 同步 | **multipart/form-data**(file+model) | JSON/text/srt/vtt;gpt-4o-transcribe 支持 SSE 流式 |
| Embeddings | `POST /v1/embeddings` | 同步 | JSON | JSON 浮点数组 |
| 视频 (Sora) | `POST /v1/videos` → `GET /v1/videos/{id}` → `GET /v1/videos/{id}/content` | **异步 submit/poll** | JSON(model=sora-2/pro, prompt, size, seconds) | task 对象:`id`+`status`(queued/in_progress/completed/failed)+`progress`;成品是**独立 content 端点下载的裸 MP4**;另有 webhook |
| 实时语音 | `wss://api.openai.com/v1/realtime` | 流式 | WebSocket JSON 事件 | base64 音频事件 |

gpt-image-1 与 dall-e 参数差异要点:quality 词表不同(`low/medium/high/auto` vs `standard/hd`)、size 枚举不同(1536x1024 vs 1792x1024)、gpt-image-1 独有 `moderation/background/output_format/output_compression/input_fidelity/stream`,dall-e-3 独有 `style` 和 `revised_prompt`。来源:[Images API 参考](https://platform.openai.com/docs/api-reference/images/createEdit)、[Videos API 参考](https://platform.openai.com/docs/api-reference/videos)、[视频指南](https://platform.openai.com/docs/guides/video-generation)。

### 2. Google Gemini(generativelanguage.googleapis.com,同域;鉴权 `x-goog-api-key` 头,非 Bearer)

| 模态 | 路径 | 同/异步 | 请求/响应 |
|---|---|---|---|
| 图像(Nano Banana) | `POST /v1beta/models/gemini-2.5-flash-image:generateContent` | 同步 | JSON,`responseModalities:["TEXT","IMAGE"]`;返回 `inlineData` **base64** |
| 图像(Imagen) | `POST /v1beta/models/imagen-4.0-*:predict` | 同步 | **instances/parameters 包裹结构**(非 contents),返回 base64 |
| TTS | `POST /v1beta/models/gemini-2.5-flash-preview-tts:generateContent` | 同步 | `responseModalities:["AUDIO"]`+`speechConfig`;返回 base64 PCM 24kHz |
| Embedding | `POST /v1beta/models/gemini-embedding-001:embedContent` | 同步 | JSON |
| 视频(Veo) | `:predictLongRunning` → `GET /v1beta/{operation_name}` | **异步 LRO** | 返回 operation name(**非裸 task id,是资源路径**),轮询 `done:true`,取 `generatedSamples` 文件 URI(下载还需带 key,约 2 天有效) |
| 实时语音 | Live API | WebSocket | 双向流 |

**OpenAI 兼容层**(`/v1beta/openai/`,实测抓取官方文档确认):覆盖 chat/completions、embeddings、**images/generations**(仅 prompt/model/n/size/response_format,其余静默忽略)、**videos(Sora 兼容端点,由 Veo 承接,含轮询)**、models、batch;**无 /audio/speech、/audio/transcriptions**,音频仅作为 chat 输入。来源:[OpenAI 兼容文档](https://ai.google.dev/gemini-api/docs/openai)、[图像生成](https://ai.google.dev/gemini-api/docs/image-generation)、[TTS](https://ai.google.dev/gemini-api/docs/speech-generation)、[视频](https://ai.google.dev/gemini-api/docs/video)。

### 3. 火山引擎/字节(**双域名双凭证体系,本次调研最大的分裂点**)

Ark 方舟域(ark.cn-beijing.volces.com,Bearer ARK_API_KEY,与 chat 同凭证):

| 模态 | 路径 | 同/异步 | 响应 |
|---|---|---|---|
| 图像 seedream | `POST /api/v3/images/generations` | 同步 | url 或 b64_json；同一路径同时承载文生图与图片编辑，参考图通过 `image` 字符串/数组传 URL 或 `data:image/...;base64,...`（产品侧最多 8 张），另有 `watermark/seed/guidance_scale` 等私有参数 |
| 视频 seedance | `POST /api/v3/contents/generations/tasks` → `GET .../tasks/{id}` | **异步** | task id 在 body;status 词表 `queued/running/succeeded/failed/cancelled`;成品 `content.video_url`(24h);截至 2026-08-23，`resolution`、`ratio`、`duration` 等生成参数使用顶层 JSON 字段 |

语音域(openspeech.bytedance.com,**完全独立的 appid/token/cluster 凭证,在语音技术控制台开通,Ark API key 不可用**):

| 模态 | 路径 | 请求形状 | 鉴权 |
|---|---|---|---|
| TTS v1 | `POST /api/v1/tts`、`wss://.../api/v1/tts/ws_binary` | JSON(body 内嵌 `app.appid/token/cluster`)/ WS 自定义二进制帧 | **`Authorization: Bearer;{token}`(分号!)** |
| 大模型 TTS v3 | `POST /api/v3/tts/unidirectional`(HTTP 单向流)、`wss://.../api/v3/tts/bidirection` | JSON;流式返回 JSON-lines 内 base64 分块 | **`X-Api-App-Key` + `X-Api-Access-Key` + `X-Api-Resource-Id`(如 volc.service_type.10029)+ `X-Api-Request-Id`** 四头体系 |
| 录音文件 ASR | `POST /api/v3/auc/bigmodel/submit` → `POST .../query` | JSON;**request id 由客户端生成放 header,query 时复用同一 X-Api-Request-Id** | 同上四头;**状态放在响应头 `X-Api-Status-Code`(20000000 成功/20000001 处理中)而非 body** |
| 流式 ASR | `wss://.../api/v3/sauc/bigmodel` | WS 二进制 | 同上 |

来源:[视频生成指南](https://www.volcengine.com/docs/82379/1520757)、[创建任务](https://www.volcengine.com/docs/82379/1521675)、[查询任务](https://www.volcengine.com/docs/82379/1521309)、火山语音技术文档(www.volcengine.com/docs/6561/*)。

### 4. 阿里 DashScope(dashscope.aliyuncs.com,单域名单 API key,但原生协议自成一派)

| 模态 | 路径 | 同/异步 | 备注 |
|---|---|---|---|
| 图像 wanx | `POST /api/v1/services/aigc/text2image/image-synthesis` → `GET /api/v1/tasks/{task_id}` | **强制异步:必须带 `X-DashScope-Async: enable` 头,否则 400** | 状态词表 `PENDING/RUNNING/SUCCEEDED/FAILED/CANCELED/UNKNOWN`;结果 `output.results[].url`(24h);**统一轮询端点 /api/v1/tasks/{id} 全平台共用** |
| TTS qwen-tts | `POST /api/v1/services/aigc/multimodal-generation/generation` | 同步(SSE 可流) | 返回音频 URL;cosyvoice 则走 WebSocket |
| ASR paraformer 文件转写 | `POST /api/v1/services/audio/asr/transcription` → `GET /api/v1/tasks/{task_id}` | 异步(同样 `X-DashScope-Async` 头) | 输入只能是公网 file_urls;结果是 `transcription_url` 指向 JSON 文件(**二跳下载**) |
| 实时 ASR/TTS | `wss://dashscope.aliyuncs.com/api-ws/v1/inference` | WS | **统一任务生命周期协议:run-task → task-started → continue-task → finish-task → task-finished**,task_id 客户端生成 |
| Embedding | `POST /api/v1/services/embeddings/text-embedding/text-embedding` | 同步 | input/parameters 包裹结构 |
| Rerank | `POST /api/v1/services/rerank/text-rerank/text-rerank` | 同步 | 同上 |

**compatible-mode**(`/compatible-mode/v1`):稳定覆盖 chat/completions、embeddings、files/batches;图像/语音兼容近期在扩展但不完整(wanx 主力仍走原生异步)——聚合项目(one-api 等)普遍反映音图需原生适配。来源:[阿里云百炼文档](https://help.aliyun.com/zh/model-studio/)、[实时识别(国际站)](https://www.alibabacloud.com/help/en/model-studio/real-time-speech-recognition)。

### 5. MiniMax(国内 api.minimaxi.com / 国际 api.minimax.io,**双平台 key 不互通**;部分接口要求 URL 上带 `?GroupId=`)

| 模态 | 路径 | 同/异步 | 响应 |
|---|---|---|---|
| TTS | `POST /v1/t2a_v2?GroupId={gid}` | 同步/SSE 流式 | **音频为 hex 编码字符串(非 base64!)**,`bytes.fromhex` 解码;终止块带 `extra_info` |
| TTS 实时 | `wss://api.minimax.io/ws/v1/t2a_v2` | WS | JSON 事件:task_start/task_continue/task_finish,音频仍 hex |
| 视频 | `POST /v1/video_generation` → `GET /v1/query/video_generation?task_id=` → `GET /v1/files/retrieve?file_id=` | **异步且三段式**:task_id 在 query 参数轮询,成功给 file_id,再走文件接口换 download_url | 状态词表 `Queueing/Preparing/Processing/Success/Fail`(首字母大写,与他家全不同) |
| 图像 | `POST /v1/image_generation` ※ | 同步 | url/base64 |

来源:[MiniMax 平台文档](https://platform.minimax.io/docs/api-reference/speech-t2a-v2)、[MiniMax-MCP 示例](https://github.com/MiniMax-AI/MiniMax-MCP)。域名沿革(api.minimax.chat→minimaxi.com/minimax.io)为二手信息※。

### 6. StepFun 阶跃(api.stepfun.com/v1,单域 Bearer,OpenAI 形态最贴近的国产厂)

- 图像:`POST /v1/images/generations`(step-1x 系列,支持 url/b64_json),另有 `/v1/images/image2image` 私有扩展※
- TTS:`POST /v1/audio/speech`(step-tts-mini,voice 可用复刻音色 id),返回二进制音频;复刻链路 file 上传 → `POST /v1/audio/voices` 得 voice_id
- ASR:`POST /v1/audio/transcriptions`(step-asr,multipart,json/text/srt/vtt)

来源:platform.stepfun.com/docs(官方站对无登录抓取返回 404,细节据搜索快照整理※)。

### 7. 智谱 BigModel(open.bigmodel.cn,单域 Bearer)

- 图像 cogview-4:`POST /api/paas/v4/images/generations`,**同步**返回 `data[].url`(OpenAI 形似)
- 视频 cogvideox:`POST /api/paas/v4/videos/generations` 得 id → **`GET /api/paas/v4/async-result/{id}`**(通用异步结果端点,GLM 异步 chat 也用它);状态词表 `PROCESSING/SUCCESS/FAIL`;成品 `video_result[].url`+`cover_image_url`

来源:[智谱文档](https://open.bigmodel.cn/dev/api)、[zhipuai SDK](https://github.com/MetaGLM/zhipuai-sdk-python-v4)。

### 8. Deepgram(api.deepgram.com,纯语音厂)

- `POST /v1/listen`:**裸二进制 body**(`Content-Type: audio/wav` + `--data-binary`)或 JSON `{"url":...}`;参数全部在 **query string**
- `wss://api.deepgram.com/v1/listen`:二进制帧推流,浏览器用 `Sec-WebSocket-Protocol: token, <key>` 子协议传 key;控制消息 `KeepAlive/CloseStream`
- 鉴权:**`Authorization: Token <key>`(非 Bearer)**;新版另有 `/v1/auth/grant` 换短期 Bearer

来源:[Deepgram 鉴权](https://developers.deepgram.com/docs/authenticating)、[预录音频](https://developers.deepgram.com/docs/pre-recorded-audio)、[流式](https://developers.deepgram.com/docs/live-streaming-audio)。

### 9. ElevenLabs(api.elevenlabs.io,纯语音厂)

- `POST /v1/text-to-speech/{voice_id}`:**voice_id 在路径里**;鉴权 **`xi-api-key` 自定义头**;返回裸二进制;`output_format` 在 query
- `POST /v1/text-to-speech/{voice_id}/stream`:chunked 二进制流
- `wss://.../v1/text-to-speech/{voice_id}/stream-input`:入 JSON 文本分片、出 **base64 音频 JSON**(与 REST 的裸二进制不同)

来源:[TTS API](https://elevenlabs.io/docs/api-reference/text-to-speech)、[WebSocket](https://elevenlabs.io/docs/api-reference/text-to-speech/v-1-text-to-speech-voice-id-stream-input)、[鉴权](https://elevenlabs.io/docs/api-reference/authentication)。

### 10. 聚合网关的归一化策略(两条路线)

- **SiliconFlow(api.siliconflow.cn/v1)——"多端点路线"**:保留 OpenAI 式独立端点 `/images/generations`(但图像返回 url、参数带 SD 系 `num_inference_steps/guidance_scale`)、`/audio/speech`、`/audio/transcriptions`、`/rerank`;**视频自创 `POST /video/submit` + `POST /video/status`(POST 轮询!),id 字段叫 `requestId`,状态词表 `InQueue/InProgress/Succeed/Failed`**。来源:[submit](https://docs.siliconflow.cn/cn/api-reference/videos/videos_submit)、[status](https://docs.siliconflow.cn/cn/api-reference/videos/videos_status)
- **OpenRouter——"全塞进 chat 路线"**:图像生成**没有独立端点**,走 `/api/v1/chat/completions` + `modalities:["image","text"]`,图像以 assistant message 的 `images` 数组(base64)返回;音频输入走 `input_audio` content;无独立 TTS/ASR/视频端点。来源:[图像生成](https://openrouter.ai/docs/features/multimodal/image-generation)、[音频输入](https://openrouter.ai/docs/features/multimodal/audio)

---

## 二、适配层设计要点(结论)

**1. 连接配置必须支持 per-task(按模态)独立的 base_url + 凭证,不能只按供应商建一条连接。** 铁证是火山:chat/图像/视频用 `ark.cn-beijing.volces.com` + Bearer ARK key,TTS/ASR 用 `openspeech.bytedance.com` + appid/token/cluster(v1)或 X-Api-App-Key 四头(v3),两套凭证互不可用。MiniMax 国内/国际双平台 key 不互通且 t2a 需额外 GroupId。因此供应商配置模型应是「供应商 → N 个连接档案(域名+凭证方案) → 模态路由到档案」,而非「供应商 = 一个 key」。

**2. 鉴权要抽象成可插拔策略,至少 5 种:** Bearer(多数)、`Token` 前缀(Deepgram)、自定义头(ElevenLabs xi-api-key、Gemini x-goog-api-key、火山 X-Api-* 四头)、body 内嵌凭证(火山 TTS v1 的 app 块)、以及火山 v1 的畸形 `Bearer;token`(分号)。浏览器场景还要支持 WS 子协议传 key(Deepgram)。

**3. 异步 submit/poll 没有任何两家完全一致,建议内部统一成「任务句柄」抽象,per-provider 写五元组映射:** (a) id 字段名:task_id / id / requestId / operation name(Gemini 是资源路径而非裸 id);(b) id 传递位置:URL 路径(OpenAI Sora、火山、DashScope、智谱)、query 参数(MiniMax)、POST body(SiliconFlow 的 status 竟是 POST);(c) 轮询端点:专属路径 vs 平台统一端点(DashScope `/api/v1/tasks/{id}` 和智谱 `/async-result/{id}` 跨产品复用,可整供应商共享一个 poller);(d) 状态词表五花八门:`queued/in_progress/completed`、`PENDING/RUNNING/SUCCEEDED`、`PROCESSING/SUCCESS/FAIL`、`Queueing/Processing/Success`、`InQueue/Succeed`,火山 ASR 甚至把状态放响应头 X-Api-Status-Code 数字码——必须做词表归一(建议归一为 pending/running/succeeded/failed/canceled);(e) 提交方式差异:DashScope 用 `X-DashScope-Async: enable` 头区分同异步,同一路径双语义,适配层要显式注入该头。

**4. 结果取回是 0~2 跳不等,任务完成 ≠ 拿到 bytes。** 智谱/火山/DashScope 图像:poll 响应直接含 URL(0 跳);OpenAI Sora:另调 `/videos/{id}/content` 下载裸 MP4(1 跳);MiniMax 视频:Success→file_id→files/retrieve→download_url(2 跳);DashScope ASR:结果是指向 JSON 转写文件的 transcription_url(1 跳下载再解析)。且各家 URL 有效期 24h~2 天,网关若要屏蔽差异需内置「立即转存」层。

**5. 语音类 WebSocket/二进制流是普遍现象,传输基座必须为非 HTTP-JSON 留位。** 实时 ASR 几乎全员 WS(Deepgram、火山 sauc、DashScope api-ws、OpenAI Realtime);TTS 流式三分天下:裸二进制 HTTP 流(OpenAI、ElevenLabs REST、StepFun)、SSE 内嵌编码块(MiniMax hex、火山 v3 JSON-lines base64)、WS 会话协议(ElevenLabs stream-input、MiniMax ws、DashScope run-task 生命周期、火山 bidirection)。且 WS 上的编码互不相同(base64 vs hex vs 自定义二进制帧)。建议基座抽象为「HTTP-JSON / HTTP-binary(上行 raw body 下行 chunked)/ SSE / WS」四种通道,音频编码(b64/hex/raw)作为通道之上的 codec 配置。

**6. 请求体形状至少 4 类,序列化层不能假设 JSON:** JSON(多数)、multipart(OpenAI/StepFun 的 edits 与 transcriptions,注意 `image[]` 数组字段名)、裸二进制上行(Deepgram)、以及 Gemini `instances/parameters`、DashScope `input/parameters` 这类非 OpenAI 包裹结构。火山 seedance 的当前原生协议把 `resolution`、`ratio`、`duration` 放在顶层 JSON，不能沿用历史 prompt 参数编码。

**7. 「OpenAI 兼容」在非 chat 模态上普遍是残缺子集,不能当真。** Gemini 兼容层图像仅认 5 个参数、其余静默忽略(静默忽略比报错更危险),音频端点完全缺失但 2026 年新增了 Sora 形态的 /videos;DashScope compatible-mode 音图覆盖不全;gpt-image-1 自己都不兼容 dall-e 的 response_format/quality 词表。适配层应按「(供应商, 模态, 模型)」三元组决定走兼容层还是原生协议,并对参数做白名单校验而非透传。

**8. 图像响应 b64 与 url 的分歧要在网关层归一。** 恒 b64(gpt-image-1、Gemini/OpenRouter)、恒 url(SiliconFlow、智谱、DashScope wanx)、可选(dall-e、火山 seedream、StepFun)。归一建议:网关统一对外提供 url+可选 b64,内部对「恒 b64」方源落盘/上传对象存储,对「恒 url」方源按需代理下载(兼顾 24h 过期问题)。

**9. 供应商内部的模型代际差异需要 per-model 参数 schema。** 同一端点上 gpt-image-1 与 dall-e-3 词表冲突、Gemini 图像分 generateContent 与 :predict 两套端点、火山 TTS v1(cluster 体系)与 v3(Resource-Id 体系)并存——配置模型里「模型」要能覆盖端点选择与参数 schema,不能只在供应商级配置。

**10. 幂等与追踪 id 语义不同:** 火山 v3 语音要求客户端生成 X-Api-Request-Id 且 submit/query 复用同一个(id 即任务句柄),DashScope WS 协议同样由客户端生成 task_id;而 OpenAI/智谱由服务端发号。任务句柄抽象需支持「客户端发号」与「服务端发号」两种模式。

### 关键来源汇总
- OpenAI:[Images](https://platform.openai.com/docs/api-reference/images/createEdit) / [Videos](https://platform.openai.com/docs/api-reference/videos) / [视频指南](https://platform.openai.com/docs/guides/video-generation)
- Gemini:[OpenAI 兼容层](https://ai.google.dev/gemini-api/docs/openai)(实抓确认含 /videos) / [图像](https://ai.google.dev/gemini-api/docs/image-generation) / [TTS](https://ai.google.dev/gemini-api/docs/speech-generation) / [Imagen](https://ai.google.dev/gemini-api/docs/imagen)
- 火山:[视频生成指南](https://www.volcengine.com/docs/82379/1520757) / [创建任务](https://www.volcengine.com/docs/82379/1521675) / [查询任务](https://www.volcengine.com/docs/82379/1521309) / 语音技术文档(volcengine.com/docs/6561/*)
- DashScope:[百炼文档](https://help.aliyun.com/zh/model-studio/) / [实时 ASR](https://www.alibabacloud.com/help/en/model-studio/real-time-speech-recognition) / [语音 demo 仓库](https://github.com/aliyun/alibabacloud-bailian-speech-demo)
- MiniMax:[t2a_v2](https://platform.minimax.io/docs/api-reference/speech-t2a-v2) / [MiniMax-MCP](https://github.com/MiniMax-AI/MiniMax-MCP)
- 智谱:[开放平台 API](https://open.bigmodel.cn/dev/api) / [SDK](https://github.com/MetaGLM/zhipuai-sdk-python-v4)
- Deepgram:[鉴权](https://developers.deepgram.com/docs/authenticating) / [预录](https://developers.deepgram.com/docs/pre-recorded-audio) / [流式](https://developers.deepgram.com/docs/live-streaming-audio)
- ElevenLabs:[TTS](https://elevenlabs.io/docs/api-reference/text-to-speech) / [WS](https://elevenlabs.io/docs/api-reference/text-to-speech/v-1-text-to-speech-voice-id-stream-input)
- SiliconFlow:[video/submit](https://docs.siliconflow.cn/cn/api-reference/videos/videos_submit) / [video/status](https://docs.siliconflow.cn/cn/api-reference/videos/videos_status)
- OpenRouter:[图像生成](https://openrouter.ai/docs/features/multimodal/image-generation) / [音频输入](https://openrouter.ai/docs/features/multimodal/audio)

**未确认/低置信项**:StepFun `/v1/images/image2image` 的确切参数(官方文档需登录);MiniMax `image_generation` 的完整参数与当前主推域名的官方迁移公告;DashScope compatible-mode 对 `/images` `/audio/*` 的最新覆盖边界(官方在持续扩展,接入时以 [help.aliyun.com](https://help.aliyun.com/zh/model-studio/) 当日文档为准);OpenRouter 是否已上线独立 /audio 端点(截至检索未见)。
