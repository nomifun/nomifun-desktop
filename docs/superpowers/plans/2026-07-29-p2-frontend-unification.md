# P2 前端统一 + 后端收缩 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 P0/P1 的重构在产品上可见并完成收缩：全部模型选择器按任务过滤（接 `/api/model-profiles/resolve`）、管理页升级为"模型实体行 + 连接档案"、克隆切服务端、创作工坊 TTS 入口、TS 契约镜像补全；后端删除 6 个冻结的 legacy map 列（迁移 016）、chat 路径平台特判表化、legacy STT 偏好一次性迁移。

**Architecture:** 两条泳道并行执行（控制器统一提交）：前端泳道（ui/，bun 测试）与后端泳道（crates/，cargo 测试）文件集不相交。轮次：R1 = T1(FE 契约) ∥ T2(BE 收缩)；R2 = T3(FE 选择器) ∥ T4(FE 管理页) ∥ T6(BE chat 表化)；R3 = T5(FE 媒体/语音/TTS) ∥ T7(BE 尾项+文档)。T3/T4/T5 均依赖 T1。

**Tech Stack:** React 19 + SWR + Arco（前端）；Rust axum/sqlx（后端）。分支 `dev/model-catalog-p2-20260729`（从 main 拉出）。

## Global Constraints

- 实现代理**禁止执行任何 git 写操作**（add/commit/stash/checkout）——控制器分道提交；测试命令前端 `bun run test:ui`（或 ui/ 下 bun test，以仓库现状为准）、后端 `cargo test -p <crate>`。
- wire 兼容底线：`ProviderResponse` 的 legacy map 字段继续输出（由行投影生成，T2 删列不删 wire 字段）；`CreateProviderRequest`/`UpdateProviderRequest` 的 map 入参继续接受（T2 改为直写行）。前端新代码一律读 `models_detail`/行级 API，不再读 legacy map 字段。
- 路由/名称启发式红线不回退：前端删除启发式后不得引入新的名字判断；选择器过滤只允许来自 resolve API。
- 磁盘：后端任务不得执行 `cargo clean` 以外的大规模重建操作；不创建 worktree。
- 提交信息结尾 `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`；禁止触碰 .github/workflows。
- 既有失败基线（合并 origin/main 后）：openclaw 构造测试 1 个 + nomifun-terminal 7 个（环境相关），与本分支无关。

## File Structure（泳道文件集，控制器据此分道提交）

- T1（FE）：`ui/src/common/types/provider/*`、`ui/src/common/adapter/ipcBridge.ts`、`ui/src/common/config/storage.ts`、新 `ui/src/renderer/services/TtsService.ts`
- T2（BE）：`crates/backend/nomifun-db/{migrations/016_*.sql, src/models/provider.rs, src/repository/{provider.rs, sqlite_provider.rs}, src/id_schema_contract.rs, tests/*}`、`nomifun-system/src/{provider.rs, managed_model.rs, model_fetcher/*}`、受牵连编译修复
- T3（FE）：新 `ui/src/renderer/hooks/agent/useModelsForTask.ts`、新 `ui/src/renderer/components/agent/TaskModelSelect.tsx`、各 chat 族选择器文件、IDMM/failover 组件、`ui/src/renderer/pages/guid/utils/modelUtils.ts`（删）
- T4（FE）：`ui/src/renderer/components/settings/SettingsModal/contents/*`、`ui/src/renderer/pages/settings/components/*`、`ui/src/renderer/pages/modelHub/*`、`ui/src/renderer/utils/model/providerClone.ts`（删）
- T5（FE）：`ui/src/renderer/pages/workshop/*`、`ui/src/renderer/pages/modelHub/SpeechToTextContent.tsx`、`ui/src/common/utils/{modelCapabilities.ts, imageModelAllowlist.ts}`（删）、`ui/src/common/config/imageGenerationMcpEnv.ts`（删）、workshop TTS UI
- T6（BE）：`crates/backend/nomifun-ai-agent/src/factory/nomi.rs`（+新 platform_table 模块）+ 快照测试
- T7（BE）：`nomifun-app/src/services.rs`（STT 偏好迁移）+ docs/specs + docs/handoffs 新文件

---

### Task 1（FE 契约基座）

TS 镜像补全 + 桥接端点。**Interfaces（后续任务 verbatim 消费）：**

```ts
// ui/src/common/types/provider/providerModel.ts（新）
export interface ProviderModelResponse { provider_id: string; model: string; enabled: boolean;
  sort_order: number; tasks: ModelTask[]; traits: ModelTrait[]; protocol?: string;
  connection_role?: string; params: unknown; context_limit?: number; description?: string;
  source: 'inferred' | 'user'; health?: ModelHealthStatus; health_checked_at?: number;
  created_at: number; updated_at: number }
export interface CreateProviderModelRequest { provider_id: string; model: string; enabled?: boolean;
  tasks?: ModelTask[]; traits?: ModelTrait[]; protocol?: string; connection_role?: string;
  params?: unknown; context_limit?: number; description?: string; sort_order?: number }
export interface UpdateProviderModelRequest { provider_id: string; model: string; enabled?: boolean;
  sort_order?: number; tasks?: ModelTask[]; traits?: ModelTrait[]; protocol?: string | null;
  connection_role?: string | null; params?: unknown; context_limit?: number | null; description?: string | null }
// ui/src/common/types/provider/providerConnection.ts（新）
export interface ProviderConnectionResponse { connection_id: string; provider_id: string; role: string;
  label?: string; base_url: string; auth_scheme: string; has_credentials: boolean; is_full_url: boolean;
  extra: unknown; created_at: number; updated_at: number }
export interface UpsertProviderConnectionRequest { role: string; label?: string; base_url: string;
  auth_scheme?: string; credentials?: unknown; is_full_url?: boolean; extra?: unknown }
```

- `IProvider`/`fromProviderResponse` 增加 `models_detail?: ProviderModelResponse[]`（wire→renderer 透传）；`ProviderResponse` 镜像加 `models_detail`。
- ipcBridge 新增：`providerModel.list/create/update/remove`（GET/POST /api/provider-models, POST .../update, .../delete）、`providerConnection.list/upsert/remove`（/api/providers/{id}/connections[...]）、`mode.cloneProvider`（POST /api/providers/{id}/clone）。健康检查绑定加 `task?: ModelTask` 字段（`acpConversation.checkProviderHealth` 请求类型补上）。
- 新 `TtsService.ts`：POST /api/tts（模式照抄 `SpeechToTextService.ts` 的 XHR/fetch + auth 处理），返回 `{blob: Blob, mime: string}`；错误解析 ApiResponse 错误包络。
- serde 测试沿 `providerApi.test.ts` 房式补齐（bun test）。

### Task 2（BE 收缩：迁移 016 删 6 列 + 直写行）

- 迁移 `016_drop_provider_legacy_model_columns.sql`：`ALTER TABLE providers DROP COLUMN` × 6（models, model_context_limits, model_protocols, model_descriptions, model_enabled, model_health）。SQLite DROP COLUMN 可用（sqlx/SQLite ≥3.35）；`capabilities` 列保留（前端 T4/T5 完成后另行退役，记录于 T7 文档）。
- `nomifun-db`：`Provider` 行/`CreateProviderParams`/`UpdateProviderParams` 删对应字段；`sqlite_provider.rs` 的 `sync_provider_models_tx` 从"双写"变为**唯一写**（create/update 的 map 入参直接翻译成 provider_models 行操作，逻辑保持：membership 同步/整 map 替换/首索引 wins）；delete 级联不变；受影响测试全部更新（断言从列改为行）。
- `nomifun-system`：`ProviderService` create/update 不再序列化 map 列（参数直接传给 repo 的行同步）；`row_to_response` 已经是行投影（改动极小：去掉对已删字段的引用）；`managed_model.rs`/`model_fetcher` 若读旧列则改行读（rg 排查）。`resolve_models`（api-types）读 `ProviderResponse` 的投影字段——不变。
- 全 workspace 消费者排查：`rg "\.models\b|model_enabled|model_protocols|model_context_limits|model_descriptions|model_health" crates/ --type rust -l` 中直接读 **DB 行字段**（非 wire DTO）的调用点（`knowledge_completer.rs`、`gateway/provider_support.rs`、`participant_resolver.rs`、`model_failover.rs`、`channel_settings.rs`、`reconcile_model_profiles` 等P0审查已知点）逐一切到 provider_models 行读或 ProviderService 投影。**这是本任务最大的工作量与风险面，宁可多花时间逐点验证。**
- 验收：`cargo test -p nomifun-db -p nomifun-system -p nomifun-ai-agent -p nomifun-conversation -p nomifun-gateway -p nomifun-channel -p nomifun-agent-execution` 绿 + workspace check；migration 测试断言 016 后列消失且 wire 投影不变。

### Task 3（FE 统一选择器）

- 新 hook `useModelsForTask(task: ModelTask, requiredTraits?: ModelTrait[])`：SWR 包 `modelProfile.resolve`，返回 `{groups: {provider, models: CatalogModelRef[]}[], isLoading}`（按 provider 分组 + provider 元数据 join 自 `useModelProviderList` 的 provider 列表）。
- 新组件 `TaskModelSelect`（Dropdown/Select 统一样式，含健康点、"(不可用)"禁用态、空态跳管理页——行为对齐现有 Knowledge/Companion 选择器的最佳实践）。
- 切换（P0 前端审查的清单）：`NomiModelSelector`/`useNomiModelSelection`、`GuidModelSelector`（删除 `guid/utils/modelUtils.ts` 重复实现）、`KnowledgeModelSelector`、`CompanionModelControl`、`PublicAgentModelPicker`、`CreateTaskDialog`（cron）、`useExecutionModelPool`/`GuidCollaboratorSelector`、`IdmmControl`、`IdmmSettingsContent`、`ModelFailoverContent` → 全部 task='chat'（IDMM/failover 首次获得过滤）。`getAvailableModels` 名称启发式路径删除（`useModelProviderList` 保留 provider 元数据职责）。
- 发送链守门：`ChatConversation`/SendBox 带图附件时校验所选模型 traits 含 `vision_input`（resolve with requiredTraits），不满足给 toast 提示（不阻断发送——后端会给 typed 错误，前端提示即可）。
- bun 测试：hook 的分组/过滤逻辑单测（mock ipcBridge）。

### Task 4（FE 管理页升级）

- **模型行改行级 API**：启用开关/上下文/描述/删除模型 → `providerModel.update/remove`（替代整 map PUT——修复读改写竞态）；供应商启用开关改写 provider.enabled 本身（语义修正，P0 审查问题 11）；删除模型不再手动清 5 个 map。
- **打标闭环**：`ModelModalityEditor` 对 `source==='inferred'` 的档案**展示推断值**（可见可确认，一次保存转 user）；traits 四项全部可编辑；模型行徽章含 chat。
- **高级抽屉**：per-model `protocol`（下拉：openai 族/gemini/deepgram/ark/volc + 自定义串）、`connection_role`、`params` 的 `endpoint`/`request_shape` 表单化 + 自由 JSON 编辑器。
- **健康检查传 task**（`ModelModalContent.tsx:701` 处补 `task: primaryTask(profile)`）。
- **连接档案区**：供应商卡片内新 section——列出 `providerConnection.list`，添加/编辑抽屉（role/label/base_url/auth_scheme 下拉[bearer/token/header_key:*/volc_voice]/credentials 按 scheme 动态表单[bearer→api_keys 文本域；volc_voice→app_key/access_key/resource_id 三输入]/is_full_url）；平台为 ark/volcengine 时在添加供应商完成后提示"可配置语音连接档案"（引导卡）。
- **克隆切服务端**：`cloneProviderConfig` 调用点改 `mode.cloneProvider`；删 `providerClone.ts`；克隆名沿服务端（" copy"）。
- bun 测试：providerApi 序列化测试扩展。

### Task 5（FE 媒体/语音/TTS + 启发式清场）

- 工坊 `ModelPicker`/`useGeneratorModels`：image→resolve(image_generation)∪resolve(image_edit)、video→resolve(video_generation)、text→resolve(chat)；`creationModels.ts` 的三级回退（profile>override>名称启发式）简化为 resolve 单源；创作页 provider 级 override chips 移除（改为跳转管理页打标的引导）。
- `SpeechToTextContent`：候选 = resolve(speech_recognition)；删除 `inferCloudSpeechService` 名字猜测（provider 枚举字段继续写死 openai——后端已忽略执行、仅展示）。
- 会话图像工具配置 `useConfigModelListWithImage`/`imageGenerationMcpEnv`：改 resolve(image_generation)；删 `imageModelAllowlist.ts` 与平台虚补模型逻辑；`imageGenerationMcpEnv.ts` 删除（NOMIFUN_IMG_* 死码链前端侧）。
- 删 TS 启发式双胞胎：`ui/src/common/utils/modelCapabilities.ts` + `ui/src/renderer/utils/model/modelCapabilities.ts`（消费点全部改 resolve/models_detail）。
- **工坊 TTS 入口**：GenMode 增加 'tts'（文本输入 + voice 选择[alloy/echo/fable/onyx/nova/shimmer + 自定义]、模型选择 resolve(speech_synthesis)）→ 走现有 `POST /api/creation/tasks` capability 'tts'（params: {prompt: text, voice}）；结果卡音频播放器（workshop 资产 URL）。
- bun 测试沿现有 harness。

### Task 6（BE chat 路径平台表化）

- 在 `nomifun-ai-agent/src/factory/` 新建 `platform_table.rs`：一张常量表 `PLATFORM_CHAT_RULES`（platform → {nomi_provider, url_rule, compat 覆盖}），`map_nomi_provider` 与 `resolve_nomi_url_and_compat` 改为查表（保留 new-api per-model protocol 特例逻辑）。
- **先写快照测试再改**：对全平台矩阵（`MODEL_PLATFORMS` 的 ~40 平台 key + 边界 base_url 变体）表驱动断言 `(provider, base_url, api_path, max_tokens_field, supports_image...)` 改造前后逐字节一致（红→绿证明行为不变）。
- 验收：`cargo test -p nomifun-ai-agent`（除既有 openclaw）绿。

### Task 7（BE 尾项 + 文档收官）

- legacy STT 偏好一次性迁移：boot reconcile 处（services.rs）读 `tools.speechToText`，若含带凭证的内嵌块且无 provider_id → 置 `enabled: false` 并去除内嵌块（写回偏好，log info 引导重选）；幂等。
- `capabilities` 列退役决议记录（T4/T5 后前端唯一消费者已移除 → 列成为纯写入死数据；本期不删列，记 P3 收缩项——避免同分支两次 providers 表 ALTER）。
- docs：spec 状态更新 + P2 偏差记录；`docs/handoffs/2026-07-29-model-catalog-p2.md`（交付/wire 变化/死码清单/遗留项/P3 入口：capabilities 列、多 key 轮换、dashscope/minimax/volc.tts_v3 适配器、realtime）。
- 最终验证：`cargo test --workspace --exclude nomifun-desktop --no-fail-fast` 对照基线（openclaw+terminal 之外零失败）+ `bun run test:ui` 全绿 + fmt。

## Self-Review 结论

- 覆盖 handoff P2 全部 8 项：选择器 ✓T3、连接档案 UI ✓T4、克隆 ✓T4、TS 契约 ✓T1、旧列删除+STT 迁移 ✓T2/T7、chat 表化 ✓T6、按需适配器→P3 记录 ✓T7、TTS 入口 ✓T5（超出 handoff 的用户可见增量）。
- 并行安全：T1/T2 文件集不相交；T3/T4/T5 间有共享文件风险点（ipcBridge 已在 T1 预置；`useModelProviderList.ts` T3 改、T5 只读——T5 排 R3 在 T3 后，安全）。
- 类型一致性：`ProviderModelResponse` 等以 T1 块为准，T4/T5 消费；`useModelsForTask` T3 定义、T5 消费。
