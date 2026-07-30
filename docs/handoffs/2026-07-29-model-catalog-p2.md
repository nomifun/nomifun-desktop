# 交接：P2 前端统一 + 后端收缩（2026-07-29/30）

- 分支：`dev/model-catalog-p2-20260729`（stacked on P1；commits 2840f0b0..HEAD，含 T5/T7 的收尾提交）
- 计划：`docs/superpowers/plans/2026-07-29-p2-frontend-unification.md`
- 设计：`docs/specs/2026-07-28-multimodal-model-provider-redesign.zh.md`（§6 P2 及"P2 实施偏差记录"）
- 前序交接：`docs/handoffs/2026-07-29-model-invoke-p1.md`（其"P2 入口"8 项本期全覆盖：1-4/6 实施，5 实施+STT 迁移，7/8 记 P3）

## 交付了什么

**1. 选择器统一（T3，commit 8415b135）——所有模型选择按任务过滤，名字启发式清场**

- 新 `useModelsForTask(task, requiredTraits?)` hook（SWR 包 `POST /api/model-profiles/resolve`，按 provider 分组、目录序、错误 fail-safe：resolve 失败保持 loading，消费者永远不会把失败当权威空目录去清空持久化引用）+ 新 `TaskModelSelect` 统一组件（健康点、"(不可用)"停用态、空目录跳管理页）。
- 10 处入口全部切换为 task='chat' 过滤：Nomi/Guid/Knowledge/Companion/PublicAgent/cron CreateTaskDialog/执行模型池/**IDMM 控制 + IDMM 设置 + 故障转移（三处首次获得过滤）**；`useModelProviderList` 的 `getAvailableModels` 名称启发式路径删除（hook 降为 provider 元数据源）。
- 发送链 vision 守门：带图附件且所选模型不在 resolve(chat, [vision_input]) 集合 → 每模型一次的 warning toast（不阻断，后端仍给 typed 错误）。

**2. 管理页升级（T4，commit 8415b135）——模型实体行 + 连接档案 + 服务端克隆**

- 模型行内编辑全部改行级 `/api/provider-models`（启用开关/上下文/描述/协议/排序/删除；null 显式清除的 tri-state），删除模型不再手动清 5 个 map——整 map PUT 的读改写竞态在这些路径消失。
- 打标闭环：`source='inferred'` 档案**可见**并预勾选 + "系统推断"标签，一次保存转 `user`；四个 traits 全部可编辑；任务徽章含 chat。
- per-model 高级抽屉：protocol（12 个 invoke 协议 id + 自定义）/ connection_role / `params.endpoint`/`request_shape` 表单化 + 自由 JSON。
- 连接档案区：`providerConnection.list/upsert/remove`，凭证表单按 auth_scheme 动态生成（bearer 族 api_keys / volc_voice 三元组 / 其余 raw JSON），只写不回读；ark/volcengine 平台自动展开 + 语音连接引导（role=voice 预填抽屉）。
- 克隆切服务端 `POST /api/providers/{id}/clone`（模型行/连接档案完整保留）——P0 遗留的"克隆丢标签"用户可见症状消除；客户端克隆工具删除。
- 健康检查请求携带 `task`（行 tasks 首项，profile 兜底）。

**3. TS 契约基座（T1，commit 2840f0b0）**

- `ui/src/common/types/provider/providerModel.ts` + `providerConnection.ts` 手写镜像（"keep in sync"指针注释 + wire key 集/tri-state/deny_unknown_fields 钉测）；`IProvider.models_detail` 透传。
- ipcBridge：`providerModel.*`、`providerConnection.*`、`mode.cloneProvider`；防御性修复：`mode.updateProvider` 桥接层剥除 `models_detail`（否则整 spread 调用点因后端 deny_unknown_fields 全部 400）。
- 新 `TtsService.ts`：`POST /api/tts` → `{blob, mime}`，AppError 包络解析，客户端 4096 字符/空文本守门。

**4. 后端收缩：迁移 016（T2，commit eff19c8f）——行存储成为唯一 per-model 存储**

- `016_drop_provider_legacy_model_columns.sql` 删 providers 表 6 列（models/model_context_limits/model_protocols/model_descriptions/model_enabled/model_health）；`capabilities` 列有意保留（见 P3 入口）。
- 原"双写"降为唯一写：create/update 的 map 入参直接翻译成 `provider_models` 行操作（membership 同步/整 map 替换/首索引 wins 语义不变）；wire DTO 不变（`ProviderResponse` 的 map 字段由行投影生成）。
- 全 workspace 行读切换（~20 个消费点）：knowledge/terminal-title completer、factory `provider_config`/`nomi`、gateway `provider_support`、participant_resolver、planner、model_failover、failover_seam（`stamp_model_unhealthy` 改行级 `set_health`，消除丢更新竞态）、conversation service、preset、managed_model、reconcile 等——详单见 `.superpowers/sdd/2026-07-29-p2-frontend-unification/task-2-report.md` §1.3。

**5. chat 路径平台表化（T6，commit 501899b8）**

- 新 `nomifun-ai-agent/src/factory/platform_table.rs`：`PLATFORM_CHAT_RULES` 常量表（14 行 + 默认行）承接 `map_nomi_provider`/`resolve_nomi_url_and_compat` 的散落平台特判；new-api per-model protocol 特例与 `is_full_url` 早退逐字保留。
- **220 行行为快照**（42 平台 × 5 base_url 变体 + 10 边界例）在改造前对旧实现生成、改造后字节级一致——行为零变化有测试证明。新增平台特殊路由的工作流 = 表加一行 + 快照矩阵扩一行（模块文档已写明）。

**6. 工坊媒体/语音/TTS + 前端启发式清场（T5）**

- 工坊 ModelPicker/`useGeneratorModels`：image→resolve(image_generation)∪(image_edit)、video→resolve(video_generation)、text→resolve(chat)；`creationModels.ts` 三级回退（profile>override>名称启发式）收敛 resolve 单源，创作页 provider 级 override chips 移除（改为跳管理页打标引导）。
- `SpeechToTextContent` 候选 = resolve(speech_recognition)，`inferCloudSpeechService` 名字猜测删除；`tools.imageGenerationModel` 选择器已不存在于任何页面——`useConfigModelListWithImage.ts`/`imageGenerationMcpEnv.ts`/`imageModelAllowlist.ts` 作为零引用死码删除（`NOMIFUN_IMG_*` 环境变量链从未与后端 builder 的键名匹配过）。
- **工坊 TTS 入口**：GenMode 'tts'（文本 + voice 选择 + resolve(speech_synthesis) 模型选择）→ 现有 `POST /api/creation/tasks` capability 'tts'；结果卡音频播放器。P1 打通的 TTS 链路自此有产品入口。

**7. legacy STT 偏好一次性迁移（T7）**

- boot 时 `migrate_legacy_speech_preference`（`nomifun-app/src/services.rs`，紧邻 `reconcile_model_profiles`，best-effort 不阻塞启动）：`tools.speechToText` 与旧键 `speechToText` 中"无 provider_id 且内嵌 openai/deepgram 块携带非空 api_key"的 P1 已退役形态 → `enabled: false` + 内嵌凭证块删除（其余字段保留），info 日志引导重选供应商。幂等；空 key 壳与 provider_id 模式不动。单元 + 内存库集成测试钉双向边界。
- 随手修复：`nomifun-system/src/provider_model.rs` 服务文档还在描述已死的"双写防漂移"——改为"行存储是唯一 per-model 存储，wire map 在边界翻译"。

## Wire / 行为变化

| 变化 | 说明 |
|---|---|
| providers 表 6 个 legacy map 列物理删除（迁移 016） | **wire 不变**：`ProviderResponse` 的 map 字段继续由行投影输出；create/update 的 map 入参继续接受（直接驱动行同步） |
| 重新加入 membership 的模型从列默认值起步 | 语义变化（T2 裁定）：双写期会继承残留 legacy map 条目；整 map 替换调用（前端 update 一贯发全量 map）不受影响 |
| 托管免费模型"目录缺席模型"的禁用开关不跨重启 | 无行可承载；进程内内存态仍保留，行承载的开关照常持久 |
| `stamp_model_unhealthy` 改行级 `set_health` | 消除整 map 读改写的丢更新竞态；不再 bump `providers.updated_at` |
| 故障转移候选无行 → fail-open | 与旧"缺 map 条目"语义一致（不构成排除项）；供应商存在性仍是硬门 |
| legacy STT 偏好 boot 迁移 | 一次性：disable + 去凭证 + info 引导（wire 端 `/api/stt` 行为已在 P1 改变，本期清掉存量数据形态） |
| IDMM/failover/各选择器首次 chat 过滤 | 已存非 chat 值不清除、继续生效，但不再被重新提供（failover 草稿显示"(不可用)"）；guid 选择不再包含禁用中的供应商 |
| 供应商启用开关写 `provider.enabled` 本身 | 语义修正（原先整卡开关不落 enabled 字段） |
| 克隆改服务端 | 名称后缀 " copy" 由服务端给出；模型行（含打标）与连接档案完整复制 |
| 健康检查请求带 `task?` | 后端按任务探测；省略时回退存量 profile/启发式路径 |
| `mode.updateProvider` 桥接剥 `models_detail` | 保护所有整 spread 的读改写调用点不因新增响应字段 400 |

## 删除清单（本分支）

前端：
- `ui/src/renderer/pages/guid/utils/modelUtils.ts`（guid 重复选择器实现，T3）
- `ui/src/renderer/hooks/agent/useModelProviderList.ts` 的 `getAvailableModels` 名称启发式路径（T3）
- `ui/src/renderer/utils/model/providerClone.ts` + `.test.ts`（客户端克隆，T4）
- `ui/src/common/utils/modelCapabilities.ts` + `ui/src/renderer/utils/model/modelCapabilities.ts`（TS 启发式双胞胎，T5）
- `ui/src/common/utils/imageModelAllowlist.ts`、`ui/src/common/config/imageGenerationMcpEnv.ts`（图像 allowlist/`NOMIFUN_IMG_*` 前端死码链，T5）
- `ui/src/renderer/hooks/agent/useConfigModelListWithImage.ts`（`tools.imageGenerationModel` 选择器 hook，零引用死码，T5）
- `SpeechToTextContent` 的 `inferCloudSpeechService` 名字猜测（T5）

后端：
- providers 表 6 个 per-model map 列（迁移 016）
- `managed_model.rs::parse_persisted_catalog` / `parse_model_enabled`；`provider_config.rs::resolve_model_context_limit`；`nomi.rs::uses_configured_openai_chat_base`（并入 platform_table）

## P3 入口

1. **`providers.capabilities` 列删除**（迁移 017）+ `ModelType`/`ModelCapability` 旧词表与 wire 字段退役——本期起该列为纯写入死数据（前端唯一消费者已随 T4/T5 移除；本期不删列避免同窗口二次 ALTER providers）。
2. **`model_health` map PUT 写路径关闭**：心跳健康持久化切行级写（行级 `set_health` 服务端权威已在 P0 落地），随后关闭 UI 的整 map 兼容写；可顺带评估 map 入参（create/update）整体退役与 AddModelModal 等残留 map PUT 的行级化。
3. **多 key 轮换与 per-key 健康**（P1 起保持"取第一个"）。
4. **按需适配器**：`dashscope.*`（aigc_image/asr_file/embedding/rerank）、`minimax.*`（t2a/video_jobs，hex codec + GroupId + 三段式取回）、`volc.tts_v3`（路由已声明，缺席时 NoAdapter）。
5. **realtime**：WS 传输通道 + 实时语音任务类型（§4.4 留位）。
6. **ts-rs 契约生成管线化**：P2 为手写镜像 + serde 钉测；生成化后删除"keep in sync"人工同步面。
7. 其余 P3 既定项：会话内文生图后端内置工具直调 invoke + 删三套遗留图像栈与 `tools.imageGenerationModel` 键；`TProviderWithModel` 引用化完成；Embedding/Rerank 首个消费者。

## 验证记录（2026-07-29/30）

- T2：`cargo test -p nomifun-db`（含新迁移 016 测试：15→16 列消失、014 回填行与其余字段逐字保留）+ system/api-types/gateway/channel/shell/preset/idmm/companion/conversation/agent-execution/ai-agent/cron/app 套件全绿（唯一例外 = 既有 openclaw 构造测试失败，见基线）。
- T6：220 行快照改造前后字节级一致；`cargo test -p nomifun-ai-agent` 853 通过 + 1 既有失败。
- T3/T4：`bun test --cwd ui` 全绿（T3 时点 1557 pass / 0 fail）；`tsc --noEmit` 干净；i18n 类型 `--check` 绿。
- T7：`cargo test -p nomifun-app -p nomifun-system -p nomifun-shell` 绿（新增 5 个 STT 迁移测试）；`cargo check --workspace --exclude nomifun-desktop` 干净。
- 既有失败基线（与本分支无关）：openclaw 构造测试 1 个 + nomifun-terminal 7 个（环境相关）。最终全工作区对照跑由控制器在合并前统一执行。

## 已记录的遗留小项（deferred minors，摘自 SDD ledger 与任务报告）

- T1（契约）：`UpdateProviderModelRequest` tri-state 依赖 JSON.stringify 语义（null 清除/缺省保留，`undefined` 被静默丢弃——类型文档与测试已钉）；TtsService 无独立单测；桥接 body mapper 可再防御性剥离 stray `id`。
- T2（收缩）：`list()` 排序 tie-break（sort_order,id）与 `list_for_provider()`（sort_order,model）外观不一致；托管免费模型缺席开关若产品要恢复跨重启持久，natural home 是 client-preference 而非 provider 列；failover "无行仍是候选"如需改为目录 membership 硬门是 `model_is_candidate` 一行改动。
- T3（选择器）：`useModelProviderList().providers` 不再隐含"有 chat 模型"——未来直接消费者不得做此假设；两 FE 泳道共用生成的 `i18n-keys.d.ts`，合并后如冲突重新生成一次即可。
- T4（管理页）：无 `models_detail` 的供应商行会合成只读行（行级写将 404）——016 后可编辑供应商不应出现该态；心跳健康仍整 map 兼容写（见 P3 入口 2）。
- T6（表化）：快照矩阵钉的是当日前端 `MODEL_PLATFORMS`——前端新增需特殊路由的平台 key 时须同步加表行 + 快照行；`nomifun-free-model` 的 `require_reasoning_content` 特例在 `provider_config.rs`（模型门控，表外，快照确认 resolve 本身不设置）。
- T7（迁移）：携带 provider_id 的偏好即使残留 stale 内嵌凭证块也不动（shell 侧忽略执行；如需彻底清库可在 P3 顺带扫除）。
