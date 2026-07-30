# 客服独立域 C1 批（并发 MVP + 旧面删除）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付可用的并发客服 MVP（spec §设计 C 的 C1 批）：新 `nomifun-customer-service` 域（六张 `cs_` 表、无状态并发回合、构造级只读工具白名单）、渠道接缝、花名册+绑定 UI，并整体删除旧 `public_agent` 面。

**Architecture:** 客服对话不进 Conversation/turn 体系：访客消息经渠道接缝移交客服域 → 按 `(bot,访客,chat)` 一人一线取 `cs_dialogues` 窗口 → `nomifun-ai-agent` 新增的通用一次性引擎入口 `run_one_shot_turn`（构建时只注册三个只读工具）生成回复 → 经渠道 sender 回传并落 `cs_messages`。跨访客并发（每客服信号量），同访客串行合并。安全无 fail-open：危险工具从不注册，`ExposureMode::PublicService` 运行时钳制随旧面退役。

**Tech Stack:** Rust（新 crate + sqlx 主库 `cs_` 表 + axum REST）、React/Arco（`pages/customerService`）、渠道插件体系复用。

## Global Constraints

- 构建前 PATH：`export PATH="/c/Users/developer/.cargo/bin:/c/Program Files/CMake/bin:/c/tools/nasm-2.16.03:$PATH"`。
- 迁移号**固定 `015_customer_service.sql`**（014 已被远控轨道占用；若 rebase 时 014 尚未合入不影响编号）。
- v3 规范：`id INTEGER PRIMARY KEY AUTOINCREMENT` 技术主键 + 具名 UUIDv7 业务 ID（GLOB CHECK 照抄 001 基线写法）；禁止物理 FOREIGN KEY；逻辑引用 + 索引。
- 命名族只用 `customer_service`/`cs_`/`CsXxx`；**禁止**出现 `public_agent`/`publicCompanion` 新引用。
- MVP 回复**非流式**（回合跑完一次性发送；流式分段属 C2 批）。对话上下文窗口：近 30 条且 ≤8000 字符。
- 一次性引擎回合硬超时 120s；每客服并发上限取 `cs_agents.max_concurrent`（默认 8）。
- 新 i18n 键 zh-CN/en-US 双语 + `bun run gen:i18n`；`ui-api-contract-version.txt` bump（若冲突取大者）。
- 每任务一 commit。

---

## File Map

| 文件 | 动作 | 职责 |
|---|---|---|
| `crates/backend/nomifun-db/migrations/015_customer_service.sql` | Create | 六张 cs_ 表 + 索引；DROP 旧 `idx_channel_plugins_public_agent_id`、`idx_conversations_extra_public_agent_id`；`ALTER TABLE channel_plugins DROP COLUMN public_agent_id` |
| `crates/backend/nomifun-db/src/models/customer_service.rs` + `repository/{customer_service.rs,sqlite_customer_service.rs}` | Create | 行模型 + `ICustomerServiceRepository` trait/Sqlite 实现 |
| `crates/backend/nomifun-db/src/id_schema_contract.rs` | Modify | cs 表契约收录；移除 public_agent_id 契约 |
| `crates/backend/nomifun-db/src/models/channel.rs` + `repository/sqlite_channel.rs` | Modify | 删 `public_agent_id` 字段 |
| `crates/backend/nomifun-ai-agent/src/one_shot.rs` | Create | 通用一次性引擎回合入口（见 Interfaces） |
| `crates/backend/nomifun-customer-service/`（新 crate） | Create | `lib.rs`/`service.rs`（CRUD+绑定）/`dialogue.rs`（窗口+并发+回合）/`tools.rs`（三只读工具构造）/`routes.rs` |
| `crates/backend/nomifun-channel/src/message_service.rs` | Modify | 接缝 trait `CsRouting`；删 `send_to_public_agent` 与 `public_agent_*` trait 方法 |
| `crates/backend/nomifun-app/src/{services.rs,router/*}` | Modify | 装配新域、挂 `/api/customer-service`、拆旧 public-agent 装配 |
| `crates/backend/nomifun-api-types/src/{preset.rs,provider_usage.rs,exposure.rs,agent_build_extra.rs,channel.rs}` | Modify/Delete | 枚举替换与 exposure 退役 |
| `crates/backend/nomifun-ai-agent/src/factory/nomi.rs` | Modify | 删 PublicService 钳制块（:128-142、:659、:702、:1052、:2420） |
| `ui/src/renderer/pages/customerService/**`（新）/`pages/publicCompanion/**`（删） | Create/Delete | 花名册+详情+绑定 UI |
| `ui/src/renderer/components/layout/{Router.tsx,Sider/index.tsx,Sider/SiderNav/*}` | Modify | 路由与"服务"分组 |
| `ui/src/common/adapter/ipcBridge.ts`、`ui/src/common/types/channel/channel.ts` | Modify | 新 API 封装；删旧 publicAgent 面 |
| i18n `locales/{zh-CN,en-US}/{customerService.json(新),publicCompanion.json(删),common.json,settings.json,index.ts}` | Modify | ns 替换 |
| `crates/backend/nomifun-public-agent/` + 根 `Cargo.toml` members | Delete/Modify | 整 crate 删除 |
| `docs/guides/channels{,.zh}.md`、`CHANGELOG.md` | Modify | 文档与废弃声明 |

**Interfaces（本计划内部及 C2 批依赖）：**

```rust
// nomifun-ai-agent/src/one_shot.rs —— 通用，无任何 cs 概念
pub struct OneShotTool {
    pub name: String, pub description: String,
    pub input_schema: serde_json::Value,
    pub handler: Arc<dyn Fn(serde_json::Value) -> futures::future::BoxFuture<'static, Result<String, String>> + Send + Sync>,
}
pub struct OneShotTurnRequest {
    pub provider: nomifun_common::ProviderWithModel,
    pub system_prompt: String,
    pub history: Vec<(String /*"user"|"assistant"*/, String)>,
    pub user_text: String,
    pub tools: Vec<OneShotTool>,
    pub timeout_secs: u64,
}
pub async fn run_one_shot_turn(services: &OneShotDeps, req: OneShotTurnRequest) -> Result<String, AppError>;
// OneShotDeps = 该 crate 内已有的 provider 凭证解析所需依赖集合，探索后固化（见 Task 3 Step 1）

// nomifun-channel 接缝（nomifun-app 注入实现）
#[async_trait] pub trait CsRouting: Send + Sync {
    async fn binding_for(&self, channel_plugin_id: &str) -> Option<String /*cs_agent_id*/>;
    async fn handle_visitor_message(&self, cs_agent_id: &str, channel_plugin_id: &str,
        channel_user_id: &str, chat_id: &str, text: &str) -> Result<String /*回复文本*/, String /*给访客的失败提示*/>;
}
```

REST（`/api/customer-service`）：`GET|POST /agents`、`GET|PATCH|DELETE /agents/{cs_agent_id}`、`GET|PUT /agents/{cs_agent_id}/bindings`（PUT body `{channel_plugin_ids:[…]}` 全量替换）、`GET|POST /notes`、`PATCH|DELETE /notes/{cs_note_id}`、`GET /dialogues?cs_agent_id=`、`GET /dialogues/{cs_dialogue_id}/messages`。

---

### Task 1: 迁移 015 + cs 行模型 + 仓储 + 契约

- [ ] **Step 1**：读 `001_v3_baseline.sql` 中 `conversations` 与 `channel_plugins` 的建表段，照抄 UUIDv7 GLOB CHECK 与索引风格，写 015：

```sql
CREATE TABLE cs_agents (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  cs_agent_id TEXT NOT NULL UNIQUE CHECK (cs_agent_id GLOB '[0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]-*'),
  name TEXT NOT NULL, greeting TEXT NOT NULL DEFAULT '', persona TEXT NOT NULL DEFAULT '',
  service_policy TEXT NOT NULL DEFAULT '', provider_id TEXT, model TEXT,
  knowledge_base_ids TEXT NOT NULL DEFAULT '[]', enabled INTEGER NOT NULL DEFAULT 1,
  max_concurrent INTEGER NOT NULL DEFAULT 8, audit_retention_days INTEGER NOT NULL DEFAULT 30,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
);
CREATE TABLE cs_channel_bindings ( id INTEGER PRIMARY KEY AUTOINCREMENT,
  cs_agent_id TEXT NOT NULL, channel_plugin_id TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL );
CREATE INDEX idx_cs_channel_bindings_agent ON cs_channel_bindings(cs_agent_id);
CREATE TABLE cs_dialogues ( id INTEGER PRIMARY KEY AUTOINCREMENT,
  cs_dialogue_id TEXT NOT NULL UNIQUE CHECK (cs_dialogue_id GLOB '…同上…'),
  cs_agent_id TEXT NOT NULL, channel_plugin_id TEXT NOT NULL,
  channel_user_id TEXT NOT NULL, chat_id TEXT NOT NULL,
  state TEXT NOT NULL DEFAULT 'open' CHECK (state IN ('open','closed')),
  created_at INTEGER NOT NULL, last_activity INTEGER NOT NULL );
CREATE UNIQUE INDEX idx_cs_dialogues_identity ON cs_dialogues(channel_plugin_id, channel_user_id, chat_id);
CREATE TABLE cs_messages ( id INTEGER PRIMARY KEY AUTOINCREMENT,
  cs_message_id TEXT NOT NULL UNIQUE CHECK (…), cs_dialogue_id TEXT NOT NULL,
  role TEXT NOT NULL CHECK (role IN ('visitor','agent','system')),
  content TEXT NOT NULL, created_at INTEGER NOT NULL );
CREATE INDEX idx_cs_messages_dialogue ON cs_messages(cs_dialogue_id, id);
CREATE TABLE cs_notes ( id INTEGER PRIMARY KEY AUTOINCREMENT,
  cs_note_id TEXT NOT NULL UNIQUE CHECK (…), cs_agent_id TEXT,  -- NULL=全体客服共享
  kind TEXT NOT NULL DEFAULT 'faq', content TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL );
CREATE TABLE cs_audit_events ( id INTEGER PRIMARY KEY AUTOINCREMENT,
  cs_agent_id TEXT NOT NULL, kind TEXT NOT NULL, platform TEXT NOT NULL DEFAULT '',
  detail TEXT NOT NULL DEFAULT '', created_at INTEGER NOT NULL );
CREATE INDEX idx_cs_audit_agent_time ON cs_audit_events(cs_agent_id, created_at);
DROP INDEX IF EXISTS idx_channel_plugins_public_agent_id;
DROP INDEX IF EXISTS idx_conversations_extra_public_agent_id;
ALTER TABLE channel_plugins DROP COLUMN public_agent_id;
```

（GLOB CHECK 省略号处照抄基线 36 位小写 v7 完整模式。）
- [ ] **Step 2**：`models/customer_service.rs` 行模型六个 struct（字段与列一一对应，`knowledge_base_ids` 存 JSON 字符串、模型层 `Vec<String>` 转换）；`repository/customer_service.rs` 定义 `ICustomerServiceRepository`（agents CRUD、bindings 全量替换/按 plugin 查、dialogue get_or_create（按三元组 UNIQUE upsert）、append_message、recent_messages(dialogue_id, limit, char_budget)、notes CRUD、audit insert/cleanup）；`sqlite_customer_service.rs` 实现。
- [ ] **Step 3**：`models/channel.rs`/`sqlite_channel.rs` 删 `public_agent_id` 字段与读写；`id_schema_contract.rs` 收录六张新表、删除 public_agent_id 契约（跑契约测试驱动改动）。
- [ ] **Step 4**：`cargo test -p nomifun-db` 全绿 → commit `feat(db): customer service tables, drop channel public_agent binding column`。

### Task 2: 新 crate 骨架 + service CRUD（TDD）

- [ ] **Step 1**：`cargo new --lib crates/backend/nomifun-customer-service`，根 `Cargo.toml` members 加入；依赖 nomifun-common/nomifun-db/nomifun-api-types + async 基建（照抄 nomifun-companion 的依赖风格）。`nomifun-common/src/id.rs` 加 `CsAgentId/CsDialogueId/CsMessageId/CsNoteId`（照抄既有 id 宏用法）。
- [ ] **Step 2**：`service.rs`：`CustomerServiceService`（持 repo Arc）实现 agents/notes/bindings 的 CRUD 校验逻辑（name 非空、max_concurrent 1..=64、reclassify enabled 布尔、绑定的 plugin id 存在性由调用方 route 层查渠道仓储）。内存 SQLite 单测：建/改/停用 agent、绑定唯一性（同 bot 重绑替换）、notes 作用域（共享+私有查询合并）。
- [ ] **Step 3**：全绿 → commit `feat(customer-service): domain crate with agent/notes/binding services`。

### Task 3: `run_one_shot_turn`（nomifun-ai-agent 内的通用一次性引擎）

- [ ] **Step 1（探索并固化 OneShotDeps）**：读 `factory/nomi.rs` 的引擎构造路径（provider 凭证如何解析成 nomi 引擎配置）与 `nomifun-companion/src/learner.rs` 的模型调用方式，二选一作为底座：优先复用引擎（支持工具循环）；把所需依赖集合固化为 `pub struct OneShotDeps {…}` 并在本文件顶注释记录选型理由。**接口签名必须与 File Map 上方 Interfaces 完全一致。**
- [ ] **Step 2**：实现：构造隔离引擎会话（无技能、无 MCP、无文件系统工具、无工作区——工具表**只含** `req.tools`），跑 tool-loop 至终止或 `timeout_secs`（tokio::time::timeout 包裹，超时返回 `AppError::Internal("one-shot turn timed out")`），返回最终文本。
- [ ] **Step 3**：单测（mock provider 或该 crate 既有测试基建）：工具白名单断言——引擎注册表内工具名恰等于传入集合；超时路径返回错误。全绿 → commit `feat(ai-agent): generic one-shot engine turn with construction-time tool whitelist`。

### Task 4: 客服回合执行器（窗口/并发/工具/审计）

- [ ] **Step 1**：`tools.rs`：三个 `OneShotTool` 构造器——`knowledge_search`（调 `KnowledgeService::search_bases(kb_ids, query, limit)`，kb_ids 来自 cs_agent 配置）、`knowledge_read`（按 hit 路径读文档内容，复用 knowledge 服务读 API）、`cs_notes_search`（repo notes 按关键词 LIKE + enabled 过滤，MVP 不做 FTS）。输入 schema 均为 `{query: string}`／`{path: string}`。
- [ ] **Step 2**：`dialogue.rs`：`CsDialogueEngine`（持 repo、KnowledgeService、OneShotDeps、`DashMap<CsAgentId, Arc<Semaphore>>`、`DashMap<CsDialogueId, Arc<Mutex<PendingBuffer>>>`）。`handle_visitor_message` 流程：get_or_create dialogue → 若该 dialogue 锁被持有则把 text 推入 pending 并返回特殊值"已合并"（调用方不发消息）→ 否则持锁：合并 pending、落访客消息、取窗口（30 条/8000 字符）、组装系统提示（persona + service_policy + greeting 规则 + "你只能依据知识库与客服笔记回答，不确定就说明并建议联系主人"）、`acquire` 信号量、`run_one_shot_turn`、落回复消息、audit insert（kind="turn"）、返回文本；失败路径返回给访客的固定提示（"暂时无法回复，请稍后再试"）并 audit kind="turn_error"。
- [ ] **Step 3**：并发集成测试（内存库 + stub OneShot——把 `run_one_shot_turn` 调用点抽成 crate 内 trait `TurnRunner` 以便注入 stub）：①两个不同访客并发调用互不阻塞（stub 里用 barrier 证明重叠执行）；②同访客第二条消息在第一回合期间到达 → 被合并进下一窗口且只产生一条回复；③信号量=1 时两访客串行。全绿 → commit `feat(customer-service): stateless concurrent dialogue engine`。

### Task 5: 渠道接缝 + 旧路径删除

- [ ] **Step 1**：`nomifun-channel`：新增 `CsRouting` trait（Interfaces 定义）；`ChannelMessageService` 加 `cs_routing: Option<Arc<dyn CsRouting>>`；`send_to_agent`（:261-277）开头：`if let Some(cs) = &self.cs_routing && let Some(aid) = cs.binding_for(plugin_id).await { let reply = cs.handle_visitor_message(…).await; → 直接经 sender 发送 reply（或失败提示）并 return }`。message_loop 的 busy 守卫对 cs 绑定 bot 跳过（客服域自己管并发）。
- [ ] **Step 2**：删除 `send_to_public_agent`、`create_public_agent_conversation`、trait 方法 `public_agent_servable/exists/name/model`、`record_public_agent_turn` 及全部调用点与测试；`apply_channel_agent_context` 中 public_agent 分支删除。
- [ ] **Step 3**：渠道单测：绑定 bot 的消息走 CsRouting stub 且不触达 conversation 路径；未绑定 bot 行为与改动前一致（伙伴路径回归用例）。全绿 → commit `feat(channel): route bound bots to customer-service seam, remove public-agent path`。

### Task 6: REST + nomifun-app 装配

- [ ] **Step 1**：`routes.rs` 按 Interfaces 的 REST 面实现（axum 风格照抄 nomifun-companion/routes.rs：ApiResponse 包装、业务 ID 参数）；`nomifun-app/src/services.rs` 装配 `CustomerServiceService`+`CsDialogueEngine`，实现 `CsRouting` 适配器注入渠道；`router` 挂 `/api/customer-service`；**拆除** `public_agent_service` 装配（services.rs:2282-2286）、路由挂载（router/routes.rs:23,640）、state 适配器（router/state.rs:731-748 一带）。
- [ ] **Step 2**：装配审计测试若存在 public-agent 断言则更新；`cargo check -p nomifun-app` 通过 → commit。

### Task 7: 旧面全清（crate/api-types/exposure/factory）

- [ ] **Step 1**：删 `crates/backend/nomifun-public-agent/` 目录 + 根 Cargo.toml members 项。
- [ ] **Step 2**：api-types：`PresetTarget::PublicCompanion` 删除（preset service 的 target_strings/parse_target 同步；parse_target 对旧值 `"public_companion"` 返回 None 属既有降级语义，CHANGELOG 记录）；`ProviderUsageFeature::PublicCompanion` → `CustomerService`（wire 值 `"customerService"`）；`channel.rs` wire 字段 `public_agent_id` 删除。
- [ ] **Step 3**：exposure 退役：`grep -rn "ExposureMode\|exposure" crates ui --include=*.rs --include=*.ts` 逐点核对——已核实消费方仅 `factory/nomi.rs` 与 api-types/agent_build_extra；若 `TrustedRemote/Private` 无其他语义消费则整模块删除（`exposure.rs`、lib.rs 导出、agent_build_extra 的 exposure 与 public_agent_id 字段、factory/nomi.rs 的钳制块 :128-142 与 :659/:702/:1052/:2420 相关段、`SAFE_PUBLIC_SERVICE_TOOLS`）；若有残余消费则仅删 `PublicService` 变体与钳制路径并在 commit message 说明。
- [ ] **Step 4**：`cargo check --workspace` 通过、受影响 crate 测试全绿 → commit `refactor!: remove public-agent domain and ExposureMode clamp`。

### Task 8: 前端（新 UI + 旧面删除 + 侧边栏）

- [ ] **Step 1**：ipcBridge：删 `publicAgent` 对象与 `IPublicAgent*` 类型（:5318-5505）及 channel 类型里的 public_agent_id；新增 `customerService` 封装（agents/bindings/notes/dialogues 按 REST 面）。`ui/src/common/types/channel/channel.ts:25` 字段删除。
- [ ] **Step 2**：新 `pages/customerService/`：花名册页（列表卡片 + 创建 Modal：名称/模型选择（复用 modelHub 选择器组件）/知识库多选（复用 knowledge 选择组件）/问候语/人设/策略/并发上限）+ 详情页（编辑各节 + 绑定管理：列出渠道 bot 复选绑定 + cs_notes 简表 CRUD）。路由 `/customer-service`、`/customer-service/:cs_agent_id`（Router.tsx 替换旧 :249-251）。
- [ ] **Step 3**：删 `pages/publicCompanion/` 整目录、`SiderPublicServiceEntry.tsx`；Sider：新 `siderSection.services`（zh「服务」/en「Services」）分组置于「增强工具」组之后，含「客服」入口（Headset 图标沿用）；`providerInUse.ts:41` 深链改 `/customer-service`（feature key `customerService`）+ 同步其测试。
- [ ] **Step 4**：i18n：新 `customerService.json`（zh/en）注册进 `locales/*/index.ts`；删 `publicCompanion.json` 与注册、`common.json` 的 `siderSection.publicService`、`settings.json` 两个旧键（presetTargetPublicCompanion、providerInUse.publicCompanion→customerService）；`bun run gen:i18n`。
- [ ] **Step 5**：`bun test`（Sider 两个结构测试 + providerInUse.test + 新页面基础结构测试）与 `bun run check:i18n` 全绿 → commit `feat(ui): customer service pages, retire public companion surfaces`。

### Task 9: 收尾核对与文档

- [ ] **Step 1**：`grep -rn "public_agent\|PublicAgent\|publicCompanion\|public_companion\|public-agents\|public-companions\|PublicService" crates/ ui/ --include=*.rs --include=*.ts --include=*.tsx --include=*.json` 清零（`docs/superpowers/` 历史文档除外）；残留逐个清。
- [ ] **Step 2**：`ui-api-contract-version.txt` bump；`docs/guides/channels{,.zh}.md` 的对外伙伴段改写为客服；CHANGELOG 加 BREAKING 条目（旧对外伙伴配置废弃，需在"服务→客服"重建；旧 preset 的 public_companion target 失效）。
- [ ] **Step 3**：全量 `cargo test --workspace`（时间过长时至少 nomifun-db/channel/customer-service/app/ai-agent 五个 crate）+ `bun test` + `cargo check --workspace` → commit `docs: customer service migration notes`。
