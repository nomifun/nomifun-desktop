# 桌面伙伴记忆与召唤、客服独立域重写、远程控制硬化 —— 综合设计

日期：2026-07-29（2026-07-30 修订：C 节由"改文案"重写为"客服独立域"）
状态：已与用户逐节确认（A/B/C/D 四节均已批准；B 节修订为"确认式记忆回写"；C 节经两轮讨论最终定为独立域重写并获用户确认）

## 背景与目标

用户对产品形态清晰、对技术实现成本纠结，经逐项澄清后本轮设计覆盖四个可独立交付的子项：

- **A. 伙伴记忆系统升级**：检索从 SQL LIKE 升级到全文检索，补齐归类整理与批量编辑能力；"无限记忆" = 永不物理删除（活跃层半衰期退热 + 归档层可检索可恢复）。
- **B. 会话中"召唤伙伴"**：把伙伴的技能与按需选择的记忆注入普通工作会话——召唤的是能力，不是人格。并以"软引导 + 一键分流"承接"用伙伴 coding"的诉求。
- **C. 客服（Customer Service）独立域重写**：原"对外伙伴"废弃，新建与桌面伙伴零耦合的 `customer_service` 域——全新 crate、全新表、无状态并发执行模型（支持群聊多访客并发）、构造级只读安全（危险工具从不注册），侧边栏入口移入灰体分组"服务"（置于"增强工具"之后）。旧 `public_agent` 代码与数据面整体删除，存量配置不迁移。
- **D. 远程控制稳定性硬化**：忙时排队、结果回推、远程解锁、错误结构化四项全做，分两批实施。

**非目标（本轮不做）**：物理世界 I/O 扩展（视频输入、TTS 语音输出）另起一轮；记忆半衰期参数与注入预算不变；Remote 前门（/mcp）的 MCP 会话过期 401、REST busy→422 等外部 MCP 客户端体验问题记为已知问题跟进（本轮远控场景以 IM 渠道为主）；旧"对外伙伴"存量数据（side-store 配置、渠道绑定、审计 JSONL）不做迁移导入——干净断开，用户在新客服域重建。

## 现状要点（探索结论）

### 伙伴与记忆
- 伙伴域独立 crate `crates/backend/nomifun-companion`，专用库 `{data_dir}/companion/shared/memory.db`（v3 硬基线 + 启动逐表契约校验，`store.rs`）。
- 记忆六维分类（profile/preference/knowledge/episode/task/affective），带 importance/strength/pinned/scope；按类型半衰期衰减（episode 7 天 … profile 永不），strength<0.05 自动归档可恢复（`store.rs:24-32, 2101`）。
- 开聊注入：pinned 全量 + 每类 top-5、总预算 6000 字符（`companion.rs:35-36`）；检索工具 `recall_memories` 目前是 SQL LIKE 子串匹配（`companion.rs:1002-1031`），无全文/语义能力。
- 管理 UI `ui/src/renderer/pages/nomi/tabs/MemoriesTab.tsx` 已有：kind/状态过滤、搜索、分页、增删改、pin、scope 切换；缺批量操作、查重合并、命中高亮。
- 建议卡基建已有：learner 产出建议 → `companion_suggestions` 表 → SuggestionsTab 用户审阅采纳。
- 技能两层：目录勾选（per-companion `skills.enabled/disabled_auto`）+ 自进化（draft→active→archived）；物化管线 `nomifun_extension::materialize_skills_for_agent` 带 manifest 所有权追踪（`companion.rs:518-583`）。
- 会话召唤机制全仓不存在；能力外溢仅 opt-in 的记忆镜像 `bridge_to_memory_dir`。

### 可行性事实
- sqlx-sqlite 0.8.6 默认启用 `bundled`，libsqlite3-sys 0.30.1 bundled 构建无条件带 `-DSQLITE_ENABLE_FTS5`（build.rs:129），SQLite 3.46 支持 trigram 分词 → **FTS5 中文检索零新依赖**。
- 模型层已有 `ModelTask::Embedding` 识别与 `/embeddings` 分发（`model_task.rs`、`dispatch_target.rs:51`），但无向量存储与调用链路 → 向量检索作为后续可选增强，本轮只预留 schema。
- 每回合动态上下文注入基建已有：`ContextContributor` trait（`nomi-agent/src/context_contributor.rs`，带长度上限与不可信标记）。
- 远程 IM 伙伴走 PROFILE_LITE，含 conversation 域（`mcp_bridge.rs:519-526`）——任务下发工具能力已具备，限制在提示词层。

### 远程控制链路与根因（已修，待部署验证）
- 链路：微信 → `nomifun-channel`（入站 receipt 去重）→ companion 会话 turn → 伙伴用 `nomi_*` 网关工具操作其他会话 → `conversation_delivery_receipts` 幂等回执承载终值（result_ok/result_text/result_error）。
- 2026-07-28 晚事故根因（微信导出诊断包证实，失败率 75%）：`build_channel_state` 的 ConversationService 未注入 KnowledgeService → 渠道 turn 以 unbound 签名申请 workspace lease；桌面侧以 bound 签名重建同一伙伴会话 runtime 后，渠道消息全部撞 lease Conflict（owner 为本会话自己），且渠道层把一切 Conflict 误映射为"处理中"，26 分钟不自愈。已由 `6e317494`（注入共享 KnowledgeService + 装配审计测试）与 `45b990c9`（仅 Starting/Running 时映射 busy）修复。
- 仍存的系统性弱点：忙时消息直接丢弃无队列（`message_loop.rs:527-538`）；后端重启后 running 会话 fail-closed 隔离需回桌面手动处理（`orphan_recovery.rs`）；`nomi_send_to_conversation` 异步下发无完成回推；`result_error` 是 Rust Debug 字符串且"Finish 但空文本"时 result_ok=false 却无错误原因（`service.rs:8711-8721`）。

### 命名面与旧实现（C 的重写对象）

- 旧"对外伙伴"= `nomifun-public-agent` crate + `pages/publicCompanion/`（路由 `/public-companions`、REST `/api/public-agents`、i18n ns `publicCompanion`）+ side-store `public-agents/{id}/config.json` + 审计 JSONL + DB 列 `channel_plugins.public_agent_id` 与 `conversations.extra.$.public_agent_id`。引用规模：Rust 49 文件、UI 46 文件。
- 旧架构的结构性问题（重写动因）：① 客服对话寄生在 Conversation/turn 体系上，继承"同会话单回合串行 + 忙时拒绝"的硬不变量，群聊多访客场景体验差；② 知识访问走 workspace 文件挂载 + lease 权威（7-28 事故正是该机械的代价），对无状态只读客服纯属多余；③ 安全靠 `ExposureMode::PublicService` 运行时钳制（`extra.public_agent_id` 触发，`factory/nomi.rs:128-142`），属"先注册后钳掉"，存在 fail-open 风险面；④ 渠道层 `send_to_agent` 中客服与伙伴是同函数互斥分支，两域代码粘合。
- 渠道会话隔离基础可复用：`(channel_plugin_id, channel_user_id, chat_id)` 三元组一人一线（`session.rs:15-22`）、入站 receipt at-most-once 幂等（`channel_inbound_receipts`）。
- `ExposureMode` 消费方仅 `nomifun-ai-agent/factory/nomi.rs` 与 `nomifun-api-types`（已核实），客服域重写后可整体退役。

## 决策记录（用户逐项确认）

1. 范围：A+B+C+D，物理世界 I/O 另起一轮。
2. 检索选型：FTS5 先行（trigram），向量作可选增强、本轮仅预留列。
3. 遗忘语义：保留半衰期衰减；归档层升级为可检索、可浏览、一键恢复；pinned 永不衰减。
4. 召唤形态：快照勾选 + 只读 recall 工具双轨；persona 不接管。
5. 记忆回写（修订）：不放开直写——工作会话产出**候选记忆**走建议卡确认流，用户采纳才入库。
6. 防 coding 入口：软引导 + 一键分流到工作会话；用户给了工作路径则创建带 workpath 的项目会话。
7. 远控硬化：忙时排队、结果回推、远程解锁、错误结构化四项全做。
8. 客服域重写（2026-07-30 最终决策，推翻 07-29 的"只改文案"）：用户明确客服是未来独立迭代的专项能力，要求与桌面伙伴在概念与实现上零耦合、支持群聊并发、构造级安全 → 全新 `customer_service` 域重写，旧 `public_agent` 面整体删除。命名族选 `customer_service`/`cs_`；存量数据干净断开不迁移；客服记忆为 cs 域自有只读表（不复用伙伴 memory.db）。

---

## 设计 A：伙伴记忆系统升级

### A1. 存储层（memory.db）

- 新增外部内容 FTS5 虚拟表 `companion_memories_fts`（`content=companion_memories`，trigram 分词），索引 `content` 列。仓储写路径（insert/update/archive/restore/delete）同步维护；启动契约校验扩展：索引缺失或计数失步时整表重建（沿用 v3 逐表校验风格，重建幂等）。
- 统一搜索接口 `search_memories(queries: Vec<String>, kind?, scope?, status: active|archived|all, time_range?, limit)`：多查询词 OR 合并去重，BM25 相关性为主序，pinned/importance/strength 加权融合；返回命中 snippet 偏移供 UI 高亮。
- 同步预留可空列 `embedding BLOB`、`embedding_model TEXT`，本轮不读不写。memory.db 的 schema 演进走 companion store 既有机制：更新内置 SCHEMA 与逐表契约校验，对存量库做幂等 ALTER 升级（新列可空、FTS 表缺失即建），不做 hard reset。
- 半衰期、归档阈值、开聊注入预算全部不变。

### A2. 工具层

`recall_memories` 换用 `search_memories`：入参改为 `queries: string[]`（提示词引导 agent 做查询扩展，弥补全文检索的近义短板）+ `include_archived: bool`（默认 false）；返回结构化条目（memory_id/kind/created_at/archived/content）。工具注册 schema 与校验同步更新（`nomi-tools` registry 校验风格）。

### A3. UI 层（增强 MemoriesTab，不新建页面）

1. 批量操作：多选 → 批量归档/恢复/删除/改分类（六维间迁移，即"归类整理"手动路径）；
2. 整理助手：一键"查重合并"——复用 learner 的归一化相似检测（`find_similar_active`）圈出疑似重复组，LLM 生成合并文案，逐组确认后合并（被并条目归档留痕，不自动执行）；
3. 检索体验：FTS5 snippet 命中高亮；相关性/时间/重要度排序切换；
4. 归档区显性化：状态切换从下拉项升级为 active/归档 分段控件，归档行内一键恢复。

### A4. API

- `GET /api/companion/memories` 增强：q 走 FTS、status=all、sort 参数；
- 新增 `POST /api/companion/memories/batch`（archive/restore/delete/reclassify，单事务）；
- 新增合并流：`POST /api/companion/memories/merge-suggestions`（dry-run 返回分组与建议文案）+ `POST /api/companion/memories/merge`（确认执行）。

### A5. 验证

仓储单测：FTS 与主表一致性（含 update/archive/restore）、中文 trigram 命中、多查询词合并、归档检索、重建幂等；UI 按 `MemoriesTab.test.ts` 现有风格补批量/合并交互测试。

---

## 设计 B：会话中"召唤伙伴"

### B1. 数据模型

会话行 `extra.summon`（沿用 extra 标记链路模式）：

```json
{ "companion_id": "…", "memory_ids": ["…"], "skill_exclusions": ["…"], "summoned_at": "…" }
```

### B2. 注入机制

- **记忆**：ContextContributor 每回合从 memory.db 按 `memory_ids` **实时解析**内容注入"召唤的伙伴记忆"区段（预算 8000 字符，超出截断并提示）；记忆被编辑后召唤方自然跟进，会话行不复制大段内容。同时把该伙伴的 `recall_memories` 以**只读**工具挂进会话（复用 `CompanionMemorySink` scope 机制），agent 干活途中可自行补查。
- **技能**：`materialize_skills_for_agent` 把伙伴 active 技能（减去 `skill_exclusions`）物化到会话工作区 `.nomi/skills/`；manifest 所有权追踪已有，解除召唤按 manifest 卸载，不碰用户自建技能。
- **persona 不接管**：系统提示仅加一句"本会话已装载伙伴 {name} 的技能与所选记忆"。
- **agent 类型范围**：首版完整支持 `type='nomi'` 会话；ACP 会话（Claude Code 等）首版提供技能物化 + recall 工具（经 gateway MCP 桥），记忆快照区段经 ACP prelude hook（知识 prelude 同款机制）跟进，若成本超预期可降级为 ACP 仅技能+工具。

### B3. 记忆回写（确认式，用户修订项）

工作会话新增 `propose_companion_memory` 工具：agent 提交候选记忆（kind + 内容 + 理由）→ 写入 `companion_suggestions`（source=summon，溯源会话 id）→ 用户在伙伴建议卡（SuggestionsTab / 悬浮窗建议入口）确认后才真正入库。直写 `save_memory` 在召唤上下文中不注册。提示词约束：宁缺毋滥，只提长期有价值的事实/偏好/约定。

### B4. 生命周期约束

召唤/调整/解除要求会话无 active turn（与知识绑定变更同一约束模式），变更下一条消息生效——runtime 重建走既有 knowledge signature 回收同款路径，避免运行中换装载导致的竞态（workspace lease 教训）。

### B5. UI

- SendBox 工具条"召唤伙伴"按钮 → 面板三步：选伙伴 → 技能清单（默认全选 active，可逐个排除）→ 记忆选择器（复用 MemoriesTab 过滤/搜索/多选组件 + FTS5 检索，实时显示预算用量）。
- 已召唤会话在头部与侧边栏条目显示伙伴徽标；点击查看/调整/解除。
- 新 i18n key 双语言齐全 + `bun run gen:i18n`。

### B6. 伙伴侧反向分流（防 coding 误用承接）

伙伴本地模式提示词增加分流规则：识别到重型 coding/工程任务时不直接开干，提议"我帮你开个工作会话并带上相关技能/记忆"。`nomi_create_conversation` 网关能力本轮扩展两个参数：

1. `workpath`：用户给了工作路径 → 创建项目会话（`extra.custom_workspace=true` + `extra.workspace=path`），自动归入侧边栏对应 workpath 抽屉；
2. `summon`：创建时即写入 `extra.summon`（伙伴 id + 伙伴按任务预选的记忆 + 技能全集），用户可事后在召唤面板修剪。

远程（微信）场景同样适用：伙伴被明确要求派活时，可创建"带着自己能力包"的工作会话。

### B7. 验证

召唤生效/解除的 runtime 重建测试；只读边界测试（召唤上下文无 save_memory、propose 走建议卡）；技能物化/卸载 manifest 测试；E2E：召唤后会话内 recall 命中伙伴私有记忆、注入区段出现且预算截断正确。

---

## 设计 C：客服（Customer Service）独立域重写

### C0. 设计原则

- **与桌面伙伴零耦合**：不共享任何概念、表、代码路径；不使用 Conversation/turn 体系——客服对话是客服域自己的领域对象，不出现在侧边栏会话列表，不参与 turn 准入/receipt/runtime registry。
- **安全靠"构造时白名单"而非"运行时钳制"**：客服引擎会话在构建时只注册只读工具（`knowledge_search`、`knowledge_read`、`cs_notes_search`），无终端/文件写/浏览器/电脑操作/网关工具——危险能力从不进入注册表，不存在 fail-open。
- **无状态并发**：每回合按消息窗口重建轻量引擎会话，跨访客天然并发，与"同会话单回合串行"的会话域不变量无关。

### C1. 新 crate `nomifun-customer-service`

**数据模型**（主库新表，v3 规范：`id` 技术主键 + UUIDv7 业务 ID，全部 `cs_` 前缀，逻辑外键 + 登记删除策略）：

| 表 | 用途 |
|---|---|
| `cs_agents` | 客服员工：名称、问候语、人设话术、模型（provider+model）、知识库绑定（kb_id 列表）、服务策略文本、enabled、`max_concurrent`（默认 8）、审计保留天数 |
| `cs_channel_bindings` | bot ↔ 客服员工绑定（`channel_plugin_id` 逻辑引用，一 bot 至多一客服）。渠道表不再持有绑定列——绑定关系归客服域所有，渠道层经接缝询问 |
| `cs_dialogues` | 访客对话：`(channel_plugin_id, channel_user_id, chat_id)` 一人一线，state open/closed，last_activity；群聊中同群不同访客各自独立对话 |
| `cs_messages` | 对话消息（访客/客服/系统），供上下文窗口与监控回看 |
| `cs_notes` | 客服域自有只读记忆：FAQ/话术/业务事实（kind + content + enabled），主人在管理页维护，运行时对客服员工只读 |
| `cs_audit_events` | 审计入库（替代旧 JSONL side-store），按 `cs_agents.audit_retention_days` 后台清理 |

**执行层（并发的关键）**：

1. 访客消息到达 → 客服域查/建 `cs_dialogues` 行 → 取上下文窗口（近 N 条 + 字符预算，默认 30 条/8000 字符）；
2. 构建**一次性 nomi 引擎轻会话**：系统提示 = 客服人设 + 服务策略 + 问候语规则；工具注册面 = 三个只读工具；知识检索直接调 `KnowledgeService::search_bases`（内存搜索 API）——**不做 workspace 挂载、不触碰 lease 权威、不进 AgentRuntimeRegistry**；
3. 流式回复经渠道 sender 直接回传（复用渠道 stream_relay 的分段/媒体能力）→ 访客与回复消息落 `cs_messages` → 引擎会话丢弃；
4. 并发控制：每客服员工一个信号量（`max_concurrent`）防模型限流雪崩；跨访客并发、同访客串行（该访客回合执行期间新到消息合并进下一回合窗口）；
5. 幂等：复用渠道既有入站 receipt（`channel_inbound_receipts` at-most-once）；客服域不需要 conversation delivery receipt。

**渠道接缝（唯一接触点）**：`ChannelMessageService::send_to_agent` 开头的路由判断改为询问客服域接缝 trait（`cs_binding_for(channel_plugin_id) -> Option<CsAgentId>`）：命中 → 整条消息移交客服域处理并返回；未命中 → 原伙伴路径不变。旧 `send_to_public_agent` 与配套 trait 方法（`public_agent_servable/exists/name/model`、`record_public_agent_turn`）全部删除。

### C2. UI（全新 `pages/customerService/`，i18n ns `customerService`）

- 路由 `/customer-service`：客服员工花名册（建/编：模型、知识库、cs_notes、策略、问候语、并发上限）+ 对话监控（进行中/历史对话列表 + 只读转写查看）+ 审计页。
- REST：`/api/customer-service/*`（agents/dialogues/notes/audit CRUD 与查询）。
- 侧边栏：灰体分组"服务"（zh「服务」/ en「Services」，i18n key 新增 `siderSection.services`），含"客服"（Customer Service）入口，置于"增强工具"分组之后。

### C3. 旧面清除（干净断开，与 C1 同批）

删除：`nomifun-public-agent` crate 及其在 `services.rs`/`router` 的装配、`pages/publicCompanion/` 目录、`/public-companions` 路由、`/api/public-agents` REST、i18n `publicCompanion` ns 与 `siderSection.publicService`、`SiderPublicServiceEntry.tsx`、`PresetTarget::PublicCompanion` 与 `ProviderUsageFeature::PublicCompanion`（新增 `CustomerService` 占用项与深链）、`channel_plugins.public_agent_id` 列（迁移删列，schema 契约测试同步）、`conversations.extra.$.public_agent_id` 写入路径、`ExposureMode::PublicService` 钳制机制（消费方仅 nomi 工厂与 api-types，已核实；客服域不再需要运行时钳制）。

收尾核对：`grep -rn "public_agent\|PublicAgent\|publicCompanion\|public_companion\|public-agents\|public-companions" crates/ ui/ docs/` 清零（历史 specs 除外）；`ui-api-contract-version.txt` bump；`bun run gen:i18n && check:i18n`；CHANGELOG 注明旧"对外伙伴"配置废弃、需在新客服域重建。

### C4. 分批

- **C1 批**：新 crate + 表 + 渠道接缝 + 无状态并发执行 + 花名册 UI + 旧面删除（可用的并发客服 MVP）；
- **C2 批**：对话监控页、审计页、群聊 @提及应答精细化、cs_notes 管理体验、访客连发合并调优。

### C5. 验证

并发集成测试：同群多访客并行回合互不阻塞、信号量封顶、同访客串行合并；安全测试：工具注册面静态断言（恰为三个只读工具）+ E2E 注入攻击（提示注入索要 bash/write/网关工具均不可达）；渠道接缝回归：伙伴绑定路径行为不变；旧面删除回归：路由/契约/i18n 检查全绿、schema 契约测试更新后通过。

---

## 设计 D：远程控制稳定性硬化

### D0. 前置：部署验证根因修复

Mac 端部署含 `6e317494` + `45b990c9` 的构建，回归 7-28 场景：微信对话期间桌面触发知识绑定变更，确认不再自锁、空闲时 Conflict 透出真实错误。

### D1. 忙时排队（渠道层）

- 新表 `channel_pending_prompts`（v3 契约：技术主键 + `prompt_id` UUIDv7）：`channel_plugin_id, chat_id, channel_session_id, conversation_id, text, idempotency_key, state(queued|delivered|expired|cancelled), queued_at, delivered_at`。
- 入队：`message_loop` 的 per-chat busy 守卫与 `send_to_agent` 的 ConversationBusy 分支从"丢弃+提示"改为"入队+回复『已排队（第 N 位），完成后自动处理』"；入队时即生成幂等 key。
- 出队：订阅 turn 完成事件 → 该会话严格 FIFO 逐条投递（仍走幂等 receipt 全链路）；投递失败按 D4 的 retryable 判定有限重试。
- 边界：每 chat 队列上限 10 条（超出拒绝提示）；默认 30 分钟未投递标记 expired 并通知用户；IM 指令「取消排队」清空本 chat 队列（复用渠道既有指令处理）。
- 重启安全：表持久化，启动恢复 queued；若目标会话处于隔离态，投递失败消息引导用户走 D3 解锁。

### D2. 结果回推

- `nomi_send_to_conversation` 加可选参数 `notify_back: bool`（默认 false；伙伴远程提示词引导"派活"类调用设 true）。
- 登记用独立小表 `conversation_delivery_notify`（`operation_id` UNIQUE、`requester_conversation_id`、state）——不动 receipt 表的 identity-immutable 触发器红线。
- 机制：目标会话 receipt 终结（`release_and_complete_turn` 之后的完成钩子）→ 按登记向发起伙伴会话投递一条 observed background 回执消息（复用 `send_observed_background_message_with_idempotency_key`，幂等 key 派生自 operation_id）→ 伙伴生成结果摘要 → 伙伴会话绑定渠道时经 stream_relay 既有链路回传微信。与 `nomi_delegate` 的完成自动回执模式对齐。
- 防环：回执消息带 origin 标记，伙伴处理回执的 turn 中 `notify_back` 无效（origin 检查硬约束 + 提示词）。

### D3. 远程解锁

- gateway conversation 域新增 `nomi_stop_conversation(conversation_id)`：Destructive 级 → Remote/Channel surface 需确认（渠道数字决策流程已有）。行为对齐桌面 UI 手动停止/重置的同一 service 安全路径。
- `nomi_conversation_status` 增强：隔离态（durable running 无法证明终态）返回 `stuck: true` + 建议文案（"该会话因后端重启被保护性挂起，可用 nomi_stop_conversation 解除后重试"）。

### D4. 错误结构化 + 自动重试

- `conversation_delivery_receipts` 增可空列 `result_error_code TEXT`、`result_error_retryable INTEGER`（additive migration；identity/lifecycle 触发器与契约测试同步扩展，终值不可改写语义不变）。
- 生成点（`service.rs` durable_completion 处）：`RelayTerminal` → 结构化映射——`Error{code,retryable}` 直接取枚举名与 retryable；`ChannelClosed` → `channel_closed`/retryable=true；各固定取消/崩溃文案各配 code（`turn_cancelled`、`owner_task_exited`、`admission_rejected` 等）；**修复不对称**：Finish 但最终文本为空 → `empty_final_text`/retryable=false（此前 result_ok=false 却无任何错误原因）。旧 `result_error` 文本列原样保留（兼容）。
- 消费：`IdempotentMessageDelivery`/`SendMessageResponse` wire 类型加可选字段（前后端契约按仓库流程同步）；渠道对 retryable=true 的失败自动重试（最多 2 次，退避 30s/120s，仍失败回真实错误文案）；`nomi_conversation_status` 透出 code。
- 后续（本轮不做）：REST `/v1` busy 由 422 改 409 区分致命错误——涉及外部契约，随 Remote 前门体验专项处理。

### D5. 实施批次

- 第一批：D0（部署验证）+ D3（成本低、立即解决"回不了桌面就卡死"）+ D4（错误地基，供 D1/D2 重试判定复用）。
- 第二批：D1 + D2（动投递链路，依赖 D4 的 retryable 判定）。

### D6. 验证

渠道集成测试：busy→入队→turn 完成→FIFO 投递、上限/过期/取消指令；notify 回推幂等与防环；`nomi_stop_conversation` 对隔离会话的解锁 E2E；receipt 新列的触发器契约测试与 RelayTerminal 映射单测；渠道 retryable 重试的退避与封顶。

---

## 总体实施顺序建议

1. **D 第一批**（D0 部署验证 / D3 远程解锁 / D4 错误结构化，见效最快）→ 2. **A**（存储→工具→UI）→ 3. **B**（依赖 A 的搜索接口与选择器组件）→ 4. **D 第二批**（D1/D2）。
**C（客服域重写）为独立专项轨道**：与桌面伙伴零耦合，可与 A/B/D 并行推进，内部按 C1（并发 MVP + 旧面删除）→ C2（监控/审计/打磨）分批；唯一合并注意点是 C1 的渠道接缝改动与 D1（渠道忙时排队）同在 `nomifun-channel`，先落地者另一方 rebase。

B6 的 `nomi_create_conversation` 扩展与 D2 的 `nomi_send_to_conversation` 扩展都在 `caps_conversation.rs`，注意合并顺序。用户文档 `docs/guides/channels{,.zh}.md` 随 C1 更新（对外伙伴→客服）。
