# P3 遗留项收尾 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成 P2 handoff 记录的 P3 遗留项：capabilities 死列删除（迁移 017）、model_health 客户端写路径关闭、克隆可传名字、前端 legacy map 读清扫、invoke 层多 key 轮换、dashscope/minimax/volc.tts_v3 三家供应商适配器、ts-rs 契约生成覆盖 provider 新域。rerank 与 realtime 明确不做（无产品消费者，记录于收官文档）。

**Architecture:** 泳道并行：R1 = T1(BE 存储/wire 退役) ∥ T2(FE 清扫)；R2 = T3(invoke 多 key 轮换) → T4(三家适配器)（同 crate 串行）∥ 无 FE；R3 = T5(ts-rs 契约) → T6(docs+终审+全量验证)。分支 `dev/model-catalog-p3-20260730`。

**Tech Stack:** 同 P2。磁盘红线：>85% 即清 build.noindex/debug/incremental；不建 worktree。

## Global Constraints

- 实现代理禁止 git 写操作；FE 泳道只动 ui/、BE 泳道只动 crates/（docs 归 T6）；测试 `bun run test:ui`+typecheck / `cargo test -p <crate>`。
- wire 兼容语义本期允许两处**记录在案的行为变化**：① `UpdateProviderRequest.model_health` / `CreateProviderRequest.model_health` 接受但忽略（服务端探针为唯一 health 写方——反伪造收口）；② `capabilities` 请求字段接受但忽略、响应恒 `[]`（列已删）。其余 wire 不变。
- 适配器纪律不变：注册表按 (protocol, task)；变体协议依据 `docs/specs/2026-07-28-provider-protocol-variance.zh.md`（标注※的细节实现处注明"接入时需真实调用校准"）。
- 提交尾注 Co-Authored-By；不碰 .github/workflows。既有失败基线：openclaw 1 个（+terminal 套件环境性抖动）。

### Task 1（BE）：capabilities 列删除 + health 写关闭 + 克隆名字 + tie-break 统一

- 迁移 `017_drop_provider_capabilities.sql`：`ALTER TABLE providers DROP COLUMN capabilities;`（先核 001 无索引/约束引用）。
- `nomifun-db`：Provider 行/Create/UpdateProviderParams 删 capabilities 字段；`sqlite_provider.rs` 不再写它；`sync_provider_models_tx` 的 health map 应用分支删除（health 只有 `set_health` 一条写路径）；`list_for_provider` ORDER BY 统一为 `(sort_order, id)`。
- `nomifun-api-types`：`ProviderResponse.capabilities` 保留字段恒空 vec（serde 默认，注释记录退役）；Create/Update 请求字段保留但文档标注 ignored；`CloneProviderRequest { name: Option<String> }` 新类型（deny_unknown_fields）。
- `nomifun-system`：create/update 忽略 capabilities/model_health 入参（不再序列化传递）；clone 端点接受可选 body，name 提供时用之、否则 `{source} copy`；投影不再读 capabilities（恒 []）。
- 测试：017 迁移测试；PUT model_health 被忽略（写后行 health 不变）测试；clone 带名字测试；受影响既有测试更新。
- 验收：`cargo test -p nomifun-db -p nomifun-api-types -p nomifun-system` + workspace check + fmt。

### Task 2（FE）：legacy map 读清扫 + 冗余写删除 + 克隆本地化后缀

- `TaskModelSelect.tsx`/`KnowledgeModelSelector.tsx` 的 `provider.model_health` 读 → `models_detail` 行 health；`GuidModelSelector` 等残留 `p.models` 读 → models_detail 回退 models；`useGeneratorModels` hasProviders 同理。
- `ModelModalContent` 心跳后的 `updateProvider({model_health})` PUT 删除（探针已服务端持久化；改为心跳返回后 mutate 刷新读投影）。
- `IProvider.capabilities` 与 `toCreateProviderRequest` 的 capabilities 发送移除；`ModelCapability/ModelType` 类型若无消费者一并删（rg 确认）。
- 克隆调用带 `{ name: sourceName + ' ' + t('settings.providerCopySuffix') }`（键复活——上轮标记死键，现在真正使用；两语言均有）。
- 验收：bun 全绿 + typecheck。

### Task 3（BE）：invoke 多 key 轮换

- `auth.rs`：`AuthMaterial::secrets() -> Vec<String>`（api_keys 全量）；`transport.rs` 或新 `rotation.rs`：`send_with_rotation(http, build: impl Fn(&str)->RequestBuilder, material) `——对 401/403/429 依序换 key 重试（每 key 一次，全败返回最后错误；记住成功 index 进程内静态 LRU 可选——P3 不做持久化）。适配器改造：submit/poll 的请求构造经轮换助手（各适配器把"组请求"闭包化——工作量集中在统一改造 13 个适配器的发送点；机械但量大，允许引入一个 `pub(crate) async fn post_json/post_multipart/...` 家族收敛发送样板）。
- MultiHeader（volc）与 BodyEmbedded 语义：多组凭证不适用轮换（credentials 单对象）→ 轮换仅作用于 api_keys 数组型 scheme（Bearer/Token/HeaderKey/QueryKey），其余单发不变。
- 测试：wiremock 第一 key 401 → 第二 key 成功（断言两次请求 Authorization 不同）；全败 → Auth 错误；非数组 scheme 不轮换。
- 验收：`cargo test -p nomifun-model-invoke` + 消费 crate 回归（creation/shell/ai-agent smoke）。

### Task 4（BE）：dashscope / minimax / volc.tts_v3 适配器

依据 variance 文档 §4/§5/§3：
- `dashscope.images`（ImageGeneration）：POST `{base}/api/v1/services/aigc/text2image/image-synthesis` + 头 `X-DashScope-Async: enable`，body `{model, input:{prompt}, parameters:{size?, n?}}` → task_id → poll GET `{base}/api/v1/tasks/{id}`（词表 PENDING/RUNNING→Pending、SUCCEEDED→output.results[].url→Assets、FAILED/CANCELED→JobFailed）。`dashscope.embeddings`（Embedding）：input/parameters 包裹 → output.embeddings[].embedding。
- `minimax.t2a`（SpeechSynthesis）：POST `{base}/v1/t2a_v2?GroupId={connection.extra.group_id}`（extra 缺 group_id → Config 错误），body `{model, text, voice_setting:{voice_id}}`；响应 `data.audio` 为 **hex** 编码 → 解码 bytes（hex codec 助手 + 单测）；`extra_info` 忽略。
- `volc.tts_v3`（SpeechSynthesis）：POST `{base}/api/v3/tts/unidirectional`（connection role voice、volc_voice 四头 + X-Api-Request-Id 客户端发号），body `{req_params:{text, speaker: voice, model}}`（※置信度中——代码注释标注需真实调用校准）；响应 JSON-lines 逐行 `{data: base64}` 聚合 → bytes（sentinel 行/`X-Api-Status-Code` 头判终）。
- routes_table：`dashscope|alibaba` 平台 image→dashscope.images、embedding→dashscope.embeddings；`minimax` tts→minimax.t2a；volc tts_v3 已有路由。注册进 default_adapters（16 个）。
- 测试：每适配器 wiremock 全链（dashscope 异步头断言+两跳、minimax hex 解码+GroupId query、volc JSON-lines 聚合+四头）；401/失败词表映射。
- 验收：`cargo test -p nomifun-model-invoke` + workspace check。

### Task 5（BE+FE）：ts-rs 契约生成覆盖 provider 新域

- 查现有 protocolBindings 管线（`rg ts-rs crates/ -l`、生成脚本/测试如何输出到 `ui/src/common/protocolBindings/`）。给 `nomifun-api-types` 的 `ProviderModelResponse/CreateProviderModelRequest/UpdateProviderModelRequest/ProviderModelKeyRequest/ProviderConnectionResponse/UpsertProviderConnectionRequest/ModelTask/ModelTrait/ProfileSource/ModelHealthStatus/HealthStatus/CatalogModelRef/ResolveModelsRequest/ResolveModelsResponse/CloneProviderRequest` 加 `#[derive(ts_rs::TS)]` + export 注解（`serde_json::Value` 字段映射 `unknown`——ts-rs 的 `#[ts(type = "unknown")]`），按管线惯例生成到 protocolBindings。
- `ui/src/common/types/provider/{providerModel.ts, providerConnection.ts}` 改为 re-export 生成类型（保留手写 doc 注释文件头指向生成源）；`storage.ts` 的 ModelTask/ModelTrait union 改 re-export（消费面大，typecheck 驱动修）。双 Option 字段核对生成形状（`string | null | undefined`）。
- 验收：生成检查测试（cargo test 触发 ts-rs export）+ bun typecheck + 全绿。

### Task 6：docs 收官 + 终审 + 全量验证

- spec 状态行 + P3 偏差记录（rerank/realtime 不做的 YAGNI 决议、两处 wire 行为变化、volc.tts_v3 ※校准注记）；`docs/handoffs/2026-07-30-model-catalog-p3.md`（交付/wire 变化/适配器矩阵 16/后续入口：per-key 健康持久化、realtime、供应商真实调用校准清单）。
- 终审（单代理整分支）+ `cargo test --workspace --exclude nomifun-desktop --no-fail-fast` 基线对照 + bun 全绿 + fmt。

## Self-Review 结论
- 覆盖 P2 handoff P3 入口全部 7 项（capabilities 列 ✓T1、health PUT 关闭 ✓T1/T2、多 key ✓T3、三家适配器 ✓T4、ts-rs ✓T5、map 读清扫 ✓T2、rerank/realtime 记录不做 ✓T6）+ 台账小项（tie-break ✓T1、providerCopySuffix 复活 ✓T2、克隆名字 ✓T1/T2）。
- 泳道安全：T1(BE)∥T2(FE) 文件不相交（T2 删 FE capabilities 类型不依赖 T1——wire 仍接受）；T3→T4 同 crate 串行；T5 跨栈单独跑。
