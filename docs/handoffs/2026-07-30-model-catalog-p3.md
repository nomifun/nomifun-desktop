# 交接：P3 遗留项收尾（2026-07-30）

- 分支：`dev/model-catalog-p3-20260730`（stacked on P2；T1/T2 = commit 785e7662，T3/T4/T5 与 docs 收官在其上）
- 计划：`docs/superpowers/plans/2026-07-30-p3-tail-closure.md`（SDD ledger 与任务报告：`.superpowers/sdd/2026-07-30-p3-tail-closure/`）
- 设计：`docs/specs/2026-07-28-multimodal-model-provider-redesign.zh.md`（§6 P3 及"P3 实施偏差记录"）
- 前序交接：`docs/handoffs/2026-07-29-model-catalog-p2.md`（其"P3 入口"7 项处置：1/2/3/4/6 实施，5 realtime 记录不做（YAGNI，P4 既定），7 其余既定项未动仍开放）

## 交付了什么

**1. 后端收缩：迁移 017 + health 写收口 + 克隆名字（T1，commit 785e7662）**

- `017_drop_provider_capabilities.sql` 物理删除 `providers.capabilities` 列（P2 起纯写入死数据）；`nomifun-db` Provider 行/Create/UpdateProviderParams 同步删字段。
- **health 唯一写方收口**：`sync_provider_models_tx` 的 health map 应用分支删除——`provider_models.health` 只剩服务端探针 `set_health` 一条写路径；create/update 的 `model_health`/`capabilities` 入参接受但忽略（wire 兼容，反伪造）。
- `CloneProviderRequest { name: Option<String> }`：`POST /api/providers/{id}/clone` 接受可选 body，带 name 用之、缺省 `{source} copy`。
- `list_for_provider` ORDER BY 统一为 `(sort_order, id)`（P2 台账 tie-break 外观不一致项消除）。

**2. 前端清扫：legacy map 读迁移 + 冗余写删除 + 克隆本地化名（T2，commit 785e7662）**

- 新助手 `ui/src/common/utils/providerModels.ts`：`modelHealthOf`（只读 `models_detail` 行 health，不回读 legacy map）/ `modelNamesOf`（行优先、legacy `models` 回退）；P2 终审点名的四处读全部切换（TaskModelSelect / KnowledgeModelSelector / GuidModelSelector ×2 / useGeneratorModels）。
- 心跳后的 fetch-latest-then-merge + `updateProvider({model_health})` 整 map 回写删除——探针结果服务端已落行，返回后仅 `mutate()` 刷新；源码钉测 `modelHealthProbePersistence.test.ts` 锁定。
- `IProvider.capabilities`、`ModelCapability`、`ModelType` 旧词表类型删除；`providerApi.ts` 请求/响应停止映射 capabilities（wire 上 vestigial `capabilities: []` 不镜像不透传）。
- 克隆走服务端端点并带本地化名：`{ name: "<源名> " + t('settings.providerCopySuffix') }`（zh "副本" / en "Copy"，死键复活）；wire 测试锁定 body 形状。
- **收官补充（docs 波次）**：设置页"清除状态"按钮与 `clearAllHealthData` handler 删除（T1 忽略客户端 model_health 写后已成 no-op；T2 报告 Concern 1 的裁定落地），`settings.clearStatus`/`settings.healthStatusCleared` 两语言键删除并再生成 `i18n-keys.d.ts`。

**3. invoke 多 key 轮换（T3）**

- `auth.rs`：`AuthScheme::rotates()`（Bearer/TokenHeader/HeaderKey/QueryKey → true；MultiHeader → false）；`AuthMaterial::secrets()`（api_keys 全量，trim 后去空）；`apply_with_secret(rb, secret)` 单 key 附着。
- `transport.rs`：`send_with_rotation(auth, || Result<RequestBuilder>)`——401/403/429 按存储顺序换 key 重试（每 key 一次），首个非触发响应即返回；全败返回**最后一个响应**交各适配器原 `error_from_response` 分类（→ Auth / RateLimited）；非轮换 scheme / <2 key / 空 key 单发不变；传输错误不轮换。闭包签名系刻意偏差：reqwest builder/multipart Form 不可克隆，每次尝试须重建。
- 发送样板收敛为 `post_json` / `post_multipart` / `post_raw` / `get_request` 家族 + `decode_hex` codec；既有 12 适配器发送点全部过家族改造（timeout/body/parse 逻辑未动）。

**4. 三家适配器（T4）——default set 12 → 16**

- `dashscope.images`（异步）：强制 `X-DashScope-Async: enable` 头 → `output.task_id` → 平台统一 poll `GET /api/v1/tasks/{id}`；词表 PENDING/RUNNING/UNKNOWN→Pending、SUCCEEDED→`output.results[].url`、FAILED/CANCELED→JobFailed。
- `dashscope.embeddings`（同步）：input/parameters 包裹，`text_type`/`dimension` 白名单，`output.embeddings[].embedding` 按 `text_index` 重排。
- `minimax.t2a`：`POST /v1/t2a_v2?GroupId={gid}`，gid 取 `connection.extra.group_id`（缺失 → 可行动 Config 错误）；`data.audio` **hex** 解码；HTTP 200 + 非零 `base_resp.status_code` → ProviderError。
- `volc.tts_v3`：`POST /api/v3/tts/unidirectional`（voice 连接、volc 四头 + 客户端发号 `X-Api-Request-Id`）；响应 JSON-lines 逐行 `{code, data: base64}` 聚合（sentinel 行跳过、失败行 ProviderError、零音频 ParseError、`X-Api-Status-Code` 头先行判错）。
- 路由表：`dashscope|alibaba` → images/embeddings 覆盖；`minimax` → t2a（default 连接，无 role）。

**16 适配器矩阵**（`crates/backend/nomifun-model-invoke/src/adapters/mod.rs` 实读）：

| # | Adapter id | 任务 | 同/异步 | 备注 |
|---|---|---|---|---|
| 1 | `openai.images` | ImageGeneration / ImageEdit | 同步 | json + multipart 双形态 |
| 2 | `openai.videos` | VideoGeneration | 异步 poll | poll + content 二跳下载 |
| 3 | `openai.chat_text` | Chat | 同步 | invoke 侧非流式探针/工具路径 |
| 4 | `openai.embeddings` | Embedding | 同步 | |
| 5 | `openai.audio_transcriptions` | SpeechRecognition | 同步 | multipart |
| 6 | `openai.audio_speech` | SpeechSynthesis | 同步 | |
| 7 | `gemini.generate_content` | ImageGeneration / ImageEdit | 同步 | 原生 generateContent |
| 8 | `gemini.generate_text` | Chat | 同步 | |
| 9 | `deepgram.listen` | SpeechRecognition | 同步 | 裸二进制上行（post_raw） |
| 10 | `ark.images` | ImageGeneration | 同步 | |
| 11 | `ark.video_jobs` | VideoGeneration | 异步 poll | |
| 12 | `volc.asr_file` | SpeechRecognition | 异步 poll | 状态在响应头；voice 连接四头 |
| 13 | `volc.tts_v3` | SpeechSynthesis | 同步（JSON-lines 流聚合） | **P3 新增**；※需真实调用校准 |
| 14 | `dashscope.images` | ImageGeneration | 异步 poll | **P3 新增**；size 词表※ |
| 15 | `dashscope.embeddings` | Embedding | 同步 | **P3 新增** |
| 16 | `minimax.t2a` | SpeechSynthesis | 同步 | **P3 新增**；hex 解码 + GroupId |

**5. ts-rs 契约生成覆盖 provider 新域（T5）**

- 管线事实：cargo test 即生成器（`export_binding_if_changed` 模式，内容变更才写盘到 `ui/src/common/protocolBindings/`）。
- `nomifun-api-types` 15 类型加 `#[derive(ts_rs::TS)]` + export：ModelTask/ModelTrait/ProfileSource、HealthStatus/ModelHealthStatus/CloneProviderRequest、ProviderModelResponse/Create/Update/KeyRequest、ProviderConnectionResponse/UpsertProviderConnectionRequest、CatalogModelRef/ResolveModelsRequest/ResolveModelsResponse。
- 形状裁定（`tests/ts_export.rs` 断言锁定）：双 Option 三态字段 → `x?: T | null`；单 Option 请求字段刻意 `x?: T`（serde null≡absent，沿用手写镜像的更窄意图型）；`serde_json::Value` → `unknown`；i64 → `number`（非 bigint）。
- FE：`providerModel.ts`/`providerConnection.ts`/`storage.ts` 相应类型改纯 re-export，零消费面破损。**老 `providerApi.ts` 手写镜像保留**——理由见 spec P3 偏差 6（legacy map 兼容字段 + 刻意非 1:1 镜像，迁移应与 map 字段退役同批）。

## Wire / 行为变化

| 变化 | 说明 |
|---|---|
| `providers.capabilities` 列删除（迁移 017） | 请求字段接受但忽略；**响应恒 `[]`**（serde 默认，注释记录退役） |
| create/update 的 `model_health` 入参接受但忽略 | 服务端探针成为行级 health 唯一写方（反伪造收口）；map 入参的其余字段（models/context_limits 等）继续驱动行同步 |
| UI"清除状态"按钮删除 | 上项的直接后果——整 map 清空 PUT 已成 no-op；行级清除能力暂无产品诉求，如需恢复走行级 API |
| 心跳后不再整 map 回写 | 探针结果服务端已落行，FE 仅刷新读投影；读改写窗口消除（P2 偏差 3 关闭） |
| 探针传输层失败不再落 unhealthy | 原 FE 兜底写路径删除且服务端未收到探针——"探不到 ≠ 不健康"，仅 toast（T2 裁定可接受） |
| 克隆可传名字 | body `{name}` 可选，缺省 `{source} copy`；FE 发送本地化"<源名> 副本/Copy" |
| `list_for_provider` 排序 `(sort_order, id)` | 与 `list()` tie-break 统一（P2 台账项） |
| 401/403/429 多 key 顺序轮换 | 单 key 型 scheme + ≥2 key 时生效；全败错误分类不变（Auth/RateLimited）；传输错误/其他状态码不轮换 |
| `primary_secret` 空串首条目不再报错 | 取第一个非空 key（轮换重构顺带行为差，原纯错误路径不变） |
| FE `ModelType`/`ModelCapability`/`IProvider.capabilities` 删除 | 无消费者（rg 确认）；wire vestigial 字段不镜像 |

## 遗留项（各 task 报告 Concerns 汇总）

- **T2-1（已收口）**：清除状态按钮语义漂移——本收官波次已删按钮与 handler（见交付 2）。
- **T2-3**：`useModelProviderList` Google Auth 合成 provider 既有 `model: []` 笔误（应为 `models`，被 `as unknown as IProvider` 掩盖）——`modelNamesOf` 的 `?? []` 已兜住，非本期范围未动。
- **T2 记录不动的 legacy 位置**：`ModelModalContent.modelRowsFor` 无行供应商合成回退（文档化设计）；`SpeechInputButton` `model_enabled` 读、`platformAuthType` `model_protocols` 读（初始导入期 legacy）；AddPlatformModal/AddModelModal/EditModeModal 整表单 map 构造路径；`providerApi.ts`/`storage.ts` legacy map 字段类型（wire 仍带）。
- **T4（设计事实）**：minimax 路由走 default 连接而 default 连接 `extra` 恒 `{}`——用户须以行级 `connection_role` 指向带 `extra.group_id` 的连接档案，否则可行动 Config 错误。
- **T3**：计划中"可选"的成功 index 进程内 LRU 未做（每次调用都从第一个 key 试起）。
- **T5**：ts-rs `no-serde-warnings` feature 经 cargo feature 统一波及同构建的 nomifun-common/db/ai-agent（它们本无此类警告，无实际影响）。
- **P2 遗留继续开放**：rerank 有路由无适配器（`NoAdapter`）；携带 provider_id 的 STT 偏好残留 stale 内嵌凭证块未扫除；P2 入口 7 的其余既定项（会话内文生图直调 invoke、`TProviderWithModel` 引用化、Embedding/Rerank 首个消费者）。

## 后续入口

1. **per-key 健康/冷却持久化**（§8-2 P4 候选）：轮换目前无记忆——成功 index LRU 与 per-key 健康列同批设计。
2. **供应商真实调用校准清单**（差异矩阵 ※ 项，代码注释已标）：`volc.tts_v3` 的 `req_params` body 形状与 JSON-lines 聚合词表；`minimax.t2a` 的 `voice_setting`/`audio_setting` 缺省语义与 hex 编码；`dashscope.images` 的 size 词表；StepFun 系 ※ 项（image2image 私有扩展等，接入立项时先真实调用核实——variance 文档 §6 注明官方文档细节系搜索快照整理）。
3. **realtime**：WS 传输通道 + 实时语音任务类型（§4.4 留位，P4 既定）。
4. **老 `providerApi.ts` 手写镜像迁 ts-rs**：与 legacy map 兼容字段（create/update map 入参、响应 map 投影）退役同批做，顺带收敛 Add/Edit 弹窗残留整 map PUT 的行级化（P2 入口 2 尾巴）。
5. rerank / Embedding 首个消费者（知识库向量化立项即用；路由表已声明）。

## 验证记录（2026-07-30）

- T1：`cargo test -p nomifun-db -p nomifun-api-types -p nomifun-system` 1818 通过 / 0 失败（控制器直接验证；含 017 迁移测试、PUT model_health 被忽略测试、clone 带名字测试）。
- T2：`bun run test:ui` 1574 pass / 0 fail（324 文件）；`bun run typecheck` 0 错误。
- T3/T4：`cargo test -p nomifun-model-invoke` **220 通过 / 0 失败**（原 187 全保留 + 33 新增：轮换矩阵、三家适配器 wiremock 全链、hex/JSON-lines/词表纯函数）。
- T5：`cargo test -p nomifun-api-types` 647+18+17+2 通过（含生成测试与形状断言）；消费 crate smoke：shell stt/tts 17、creation 55、ai-agent provider_health 14 全绿。
- 全局：`cargo check --workspace --exclude nomifun-desktop` 干净（仅 pre-existing nomifun-app dead-code warnings）；`cargo fmt --all --check` 干净。
- docs 收官波次（按钮删除后复跑）：`bun run test:ui` 1574 pass / 0 fail；`bun run typecheck` 0 错误；`i18n-keys.d.ts` 已再生成（5180 keys）。
- 既有失败基线（与本分支无关）：openclaw 构造测试 1 个 + nomifun-terminal 套件环境性抖动。
