# 交接：P1 统一调用层 nomifun-model-invoke（2026-07-29）

- 分支：`dev/model-catalog-p1-20260729`（stacked on P0 `195f23c4`；commits 975bf80c..6c796f06 + 本文档提交）
- 计划：`docs/superpowers/plans/2026-07-29-p1-model-invoke-layer.md`
- 设计：`docs/specs/2026-07-28-multimodal-model-provider-redesign.zh.md`（§6 P1 及"P1 实施偏差记录"）
- 协议依据：`docs/specs/2026-07-28-provider-protocol-variance.zh.md`（ark/volc 语音域 §3）

## 交付了什么

**新 crate `crates/backend/nomifun-model-invoke`（底层基础能力，依赖红线 = common/api-types/db/net，不依赖任何产品 crate）**

模块清单：`lib.rs / error.rs / types.rs / call.rs / auth.rs / transport.rs / adapter.rs / routes_table.rs / resolve.rs / service.rs / adapters/{mod, openai_images, openai_videos, openai_chat_text, openai_embeddings, openai_audio, gemini, deepgram, ark, volc_voice}`。三层结构：

1. **resolve 管线**（`resolve.rs`）：providers 行 + provider_models 行 + provider_connections 行 + 平台路由表 → `ResolvedCall`（守门：enabled / task ∈ tasks / 未播种回退名称推断；连接：default = providers 行凭证，role 覆盖 = 连接档案解密 + 声明式 `AuthScheme`）；
2. **适配器注册表**（`adapter.rs` + `adapters/`）：`ProtocolAdapter`（submit/poll），查找键只允许 `(protocol_id, ModelTask)`——名字嗅探路由全部铲除；
3. **invoke / poll / probe 编排**（`service.rs`）：探针与真实调用同一条解析管线；`InvokeError` 统一错误分类（含 `JobFailed` 远端异步任务终态语义）；`InvokeError → AppError / CreationError / SttError` 映射在各消费方边界。

**12 个适配器（`default_adapters()`）**

| 适配器 id | 任务 | 来源 |
|---|---|---|
| `openai.images` | ImageGeneration + ImageEdit | 迁自 creation `adapters/openai_images.rs`（鉴权改声明式） |
| `openai.videos` | VideoGeneration | 迁自 creation `adapters/openai_video.rs`（**修复**：改走 dispatch_target，`params.endpoint` 覆盖对视频生效） |
| `openai.chat_text` | Chat | 迁自 creation `adapters/openai_chat.rs`（非流式） |
| `openai.embeddings` | Embedding | 新写 |
| `openai.audio_transcriptions` | SpeechRecognition | 迁自 shell `stt_openai.rs` |
| `openai.audio_speech` | SpeechSynthesis | 新写（JSON→裸二进制 + mime 推断） |
| `gemini.generate_content` | ImageGeneration + ImageEdit | 迁自 creation `adapters/gemini_image.rs`（**修复**：count>1 循环聚合） |
| `gemini.generate_text` | Chat | 迁自 creation `adapters/gemini_text.rs` |
| `deepgram.listen` | SpeechRecognition | 迁自 shell `stt_deepgram.rs`（Token 鉴权 + raw-body 进基座） |
| `ark.images` | ImageGeneration | 新写（火山方舟 `/api/v3/images/generations`） |
| `ark.video_jobs` | VideoGeneration | 新写（异步 submit→poll，prompt 内参数编码 `--resolution/--duration`；`params.endpoint` 覆盖生效） |
| `volc.asr_file` | SpeechRecognition | 新写（openspeech 域，`volc_voice` 四头鉴权，状态在响应头，客户端发号 request_id） |

**产品侧切换与死码删除（-1853 行）**

- creation：内嵌 `adapters/*` + `provider.rs` 整体删除（-1553 行），`CreationService` 改为 `MediaCapability → TaskRequest` 映射 + `invoke.invoke/poll`；`JobHandle` JSON 持久化进 `remote_task_id`（旧裸 id 行读取兼容，boot resume 不受影响）；守门收紧——未打标模型创建任务得到 typed `unsupported_capability` 而非打错端点。
- shell：`stt_openai.rs` + `stt_deepgram.rs` 双实现删除（-300 行），STT 协议由平台路由表决定（deepgram 平台 → `deepgram.listen`，其余 → `openai.audio_transcriptions`，模型行 protocol 可覆盖）——前端的 provider 枚举猜测字段仍被接受但被忽略。
- ai-agent：`provider_health.rs` 的 `run_modality_probe`/`minimal_json_body`/`minimal_multipart_form` 删除，非 chat 探针委托 `invoke.probe()`——**探针与真实调用自此同管线**；wire 保持"200 + unhealthy 报告"，永不变 HTTP 错误。

**TTS 端到端**：`openai.audio_speech` 适配器 + `POST /api/tts`（shell 路由，二进制响应）；创作工坊 Tts capability 从"必失败"变为可用（wiremock e2e：`/v1/audio/speech` → 任务 succeeded + audio/mpeg 产物）。

**火山双域验证件**：同一 provider（platform=ark）default 连接走方舟域（图像/视频），`provider_connections` role="voice" 档案走 openspeech 域（ASR，`volc_voice` 凭证）。端到端测试证明**同一供应商不同模态走不同域名 + 不同凭证**；删除 voice 档案后同调用得到 typed `MissingConnection`。

**逃生舱**：per-model `params.endpoint` 覆盖现对 images/videos（openai 族 + ark 族）全部生效（P0 已知的"对视频无效"缺陷已修）。

## Wire / API 变化

| 变化 | 说明 |
|---|---|
| `POST /api/tts` | **新增**：`{provider_id, model, text, voice?, format?}` → 音频字节直接回（Content-Type 按 format/响应头推断，非 ApiResponse 包络）；文本上限 4096 字符；未打标模型 → 400 |
| `POST /api/stt` legacy 内嵌 key 模式 | **退役**（wire 行为变化）：无 provider_id、且内嵌 openai/deepgram config 携带**非空 api_key** 的旧偏好 → 500 STT_UNKNOWN + 可行动消息引导重选供应商；内嵌块为空壳（api_key 为空）则不触发守卫，照旧回落 NOT_CONFIGURED 400 族（commit 4b13ece7 钉测双边界）。前端 UI 早已只写 provider_id 模式，存量旧偏好的一次性迁移列为后续改进 |
| `POST /api/agents/provider-health-check` | 内部改道 `invoke.probe()`，wire 不变（200 + healthy/unhealthy 报告，latency/message 字段照旧） |
| `POST /api/creation/tasks` | wire 形状不变；错误 kind 细化：V2v → `unsupported_capability`（原 `adapter_unavailable`）；远端异步任务失败 → `provider_error` 携带真实原因（原先可能降级为假 `timeout`） |

## 死码清单（本分支删除）

- `nomifun-creation/src/adapters/*`（mod/openai_images/openai_video/openai_chat/gemini_image/gemini_text 等）+ `nomifun-creation/src/provider.rs`
- `nomifun-shell/src/stt_openai.rs` + `nomifun-shell/src/stt_deepgram.rs`
- `nomifun-ai-agent/src/services/provider_health.rs` 中的 `run_modality_probe` / `minimal_json_body` / `minimal_multipart_form`

## P2 入口

1. 前端统一选择器接 `POST /api/model-profiles/resolve`（含健康/禁用原因/缺连接提示）；
2. 管理页连接档案 UI（按平台注册表渲染角色卡片；volc 双连接引导——语音连接未配时打了 TTS/ASR 标的模型显示禁用态而非选中后失败）；
3. 设置页克隆切换到服务端 `POST /api/providers/{id}/clone`（替换 `ui/src/renderer/utils/model/providerClone.ts`，消除克隆丢标签的用户可见症状）；
4. TS 契约生成（provider/connection/model 域纳入 ts-rs，删 TS 侧启发式双胞胎）；
5. providers 旧 6 个 JSON map 列物理删除 + legacy STT 偏好一次性迁移；
6. chat 路径平台表化（`map_nomi_provider`/`resolve_nomi_url_and_compat` 入路由表，P1 有意未动）；
7. `volc.tts_v3` / dashscope 系 / minimax 系适配器按需落地（volc.tts_v3 与 rerank 的路由已声明，缺适配器时为诚实 NoAdapter）；
8. 多 key 轮换与 per-key 健康（P1 保持"取第一个"）。

## 验证记录（2026-07-29）

- 受改 crate 套件全绿：`cargo test -p nomifun-model-invoke`（172 通过）、`-p nomifun-creation`（55 通过，e2e 行为快照不改断言）、`-p nomifun-shell`（96 通过）、`-p nomifun-ai-agent`（850 通过 + 1 个 openclaw 既有失败，见 P0 handoff）。
- 全工作区 `cargo test --workspace --exclude nomifun-desktop --no-fail-fast` 与 P0 基线（36 个既有失败）对照：初跑发现 **6 个真实回归**（nomifun-app/tests/shell_e2e.rs 的 STT 套件仍在播种已退役的内嵌凭证配置形态）——已修复（commit 4b13ece7：守卫改为仅在嵌入块携带非空凭证时拒绝、空壳回落 NOT_CONFIGURED；4 个测试移植目录模式并加双边界钉测），复审确认 140/140 全绿。修复后回归数为 **0**；36 个既有失败逐字重现（与本分支无关）。对照报告：.superpowers/sdd/2026-07-29-p1-model-invoke-layer/p1-baseline-comparison.md。

## 已记录的遗留小项（deferred minors，摘自 SDD ledger）

- T1（骨架）：`RateLimited → AppError` 的 From 映射丢弃 retry_after_ms（AppError 为单元变体）；`primary_secret` 对畸形凭证形状的边角未测；脚手架依赖在 T2 前闲置。裁定备忘：模型行 protocol 覆盖若指向未注册协议，解析层面按 Config/BadRequest 处置（T2 已落实）。
- T2（解析管线）：disabled 模型阻断探针的路径当时未测（T5/T7 补钉）；strict 凭证解析未测；key 列表全空时以 "" 继续（错误来自上游 Auth 而非本地 Config）。行为变化备忘：invoke.probe 对 disabled 模型/供应商现在报错（旧探针会照跑）——UI 需确认对 disabled 行隐藏/处理健康按钮。
- T3（OpenAI 族）：video_base 剥掉 query string（Azure `?api-version` 在 poll/content 阶段丢失——真实配置注意）；embeddings 有死 unwrap_or 且无排列校验；images 的 count>=1 不在适配器层校验（属调用方层，creation 保留 param_count）。
- T4（gemini/deepgram/transcriptions）：gemini count 循环无上限箝制（creation 的 param_count 在调用方边界保留 10 上限）；deepgram 在无 language 时总是发送 detect_language（与迁移源的外观差异）；ext_for_audio_mime 不剥 mime 参数。
- T5（编排）：TTS 探针在 openai.audio_speech 注册前 unhealthy（**已被 T8 解除**）；extra-overlay 现在作用于所有 invoke() 图像生成（有意，镜像遗留行为）；带有效 stub PNG 的探针在真实供应商上可能产生一次真实付费编辑（reachable-probe 设计）。
- T6（creation 切换）：gemini 命名模型在非 gemini 平台行上需行级 protocol 覆盖（设计内行为，名字嗅探已死，已在偏差记录披露）；V2v 错误 kind 改 `unsupported_capability`（简报裁定）；`provider_repo()` 访问器暴露面宽于所需；手工枚举的 kind 测试矩阵缺编译期完整性保障。
- T7（shell/探针切换）：legacy 内嵌 STT 配置 → 500 STT_UNKNOWN（消息可行动；一次性配置迁移列为后续改进）；协议覆盖边角上 provider 字段有外观性错配；elapsed_ms 含解析耗时。
- T8（TTS）：无专门的 probe(SpeechSynthesis) 测试；未知 format 的 mime 误标为 audio/mpeg；4096 字符临界接受侧未测。
- T9（ark）：images 的 count 被静默丢弃（ark 单图一调用的现实）；未知 poll 状态 → 永远 Pending（沿用仓内惯例）；报告测试计数差一。
- T10（volc ASR）：ark submit 对 endpoint 覆盖剥 query（与 video_base 的逐字模式有出入）；header 优先级的病态形状未穷举；poll 侧分支覆盖借道 submit 侧；submit 容忍 2000000{1,2} 状态码；is_full_url 对双端点协议语义不明。
