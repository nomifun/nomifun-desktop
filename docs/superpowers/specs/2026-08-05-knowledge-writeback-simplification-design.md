# 知识库回写简化（移除暂存 · 回写意识改手动/自动 · 删除待审）设计

- 日期：2026-08-05
- 状态：待用户复核（方向性决策已逐项确认，见 §0.2）
- 涉及仓库：`nomifun-tauri`（全部实现）
- 分支基线：`refactor/knowledge-writeback-simplification` @ `9599ddc1`（v0.3.8），从 `main` 切出
- 交付物：
  1. `docs/superpowers/specs/2026-08-05-knowledge-writeback-simplification-design.md` — 本文
  2. `docs/superpowers/plans/2026-08-05-knowledge-writeback-simplification-plan.md` — TDD 实施计划（writing-plans 阶段产出）
  3. 代码 + 随码同行改动（`migrations/025_*.sql`、README EN/ZH、`docs/guides/companions.{md,zh.md}`、`CHANGELOG.md`、`ui-api-contract-version.txt` 14→15）

## 0. 背景与目标

### 0.1 要做的三件事

知识库回写当前有两个正交维度和一个人工审阅环节，理解成本高、代码面大：

1. **回写模式**（`writeback_mode`）：`staged`（写入被约束在 `_inbox/{会话id}/`）或 `direct`（可改库正文）。
2. **回写意识**（`writeback_eagerness`）：`conservative` 或 `aggressive`，只影响抽取提示词的措辞。
3. **待审**（知识库-待审）：暂存提案的审阅/合并/丢弃界面与 API。

本次简化为：

1. **移除暂存**，回写只有"直接回写"一种落点。
2. **回写意识改为 `manual`（手动型）/ `auto`（自动型）**，且让两者真正改变行为，而非仅措辞。
3. **彻底移除待审**。

### 0.2 已确认的方向性决策

| 决策点 | 结论 | 备选与否决理由 |
|---|---|---|
| `writeback_mode` 去向 | **整列删除**，不保留单值枚举 | 开关早已由 `knowledge_bindings.writeback` 布尔列承担；留一个只有一个合法值的列就是标准的遗留隐患 |
| 外部 IM 渠道回写 | **改为直接回写**，保留 `channel_write_enabled` 开关 | 否决"一并删除渠道回写能力"（用户明确要保留该能力）；否决"渠道锁定手动型"（引入"表面覆盖挂载设置"的特例逻辑） |
| 手动型判定口径 | **在 provider 调用之前短路回合末抽取**；`knowledge_write` 工具照常注册，用户在对话中要求即可写入 | 否决"照跑抽取靠提示词约束"（每轮仍烧一次 LLM 调用，且"手动"沦为模型自觉）；否决"手动型完全禁止 Agent 写入"（"帮我记一下"这个最自然的场景就用不了了） |
| 回写意识归属层级 | **保持挂载级（`knowledge_bindings`）不动** | 同一知识库挂到不同会话/终端/伙伴上可以有不同意识，粒度本就正确；提升到知识库级会牵动挂载 UI、wire 契约与迁移，改动面显著扩大而无收益 |
| 磁盘上已有的 `_inbox/` | **文件原地不动，但删除全部 `_inbox` 特例逻辑** | 否决"迁移时删除"：知识库根可以是用户任意指定的外部绝对路径（`service.rs:968-997`，`managed:false`），`_inbox` 可能就在用户自己的 git 仓库里；且 SQL 迁移表达不了删文件，需要额外写一段在用户目录递归删除的代码。否决"合并进正文"：暂存提案是**未经用户批准**的候选，无条件合并等于替用户拍板 |
| 覆写安全 | **纳入本次**：把 `knowledge_write` 工具的直写路径统一为"读取 → 追加式合并 → CAS" | 暂存是当前唯一带 CAS 校验（`StagedProposalMetadata.base_sha256`）的写入路径；删掉它而不补，等于把"模型一次错误调用即可销毁精修文档"变成唯一路径。仓库里已有安全实现，只是工具路径没用 |
| `preset_knowledge_policy.mode` | **一并删除** | 值域 `'inherit'｜'staged'｜'direct'` 在没有"模式"维度后整体失意 |
| 自动型频率控制 | **仅提示词 + 现有内容级去重**，不新增计数机制 | `contains_markdown_block` 的块级去重覆盖"不重复记录"；"不记琐碎/临时"本质上只能靠提示词。新增真实频率上限属于本次重构之外的新功能 |

### 0.3 关键前提（已核实，非推断）

- **列在 `knowledge_bindings` 上，不在 `knowledge_bases` 上**：`migrations/001_v3_baseline.sql:1167-1171`。
- **baseline 不可编辑**：`001_v3_baseline.sql` 被 sqlx 校验和钉死，`nomifun-db/src/database.rs:221-252` 每次打开库都比对，不匹配即 `DbError::Init("database migration lineage does not match embedded migration 1")`，经 `nomifun-app/src/bootstrap/environment.rs:296-300` 变成启动失败。
- **SQLite 不能改 CHECK**：两列都带列级 CHECK（`:1168-1169`、`:1170-1171`）。
- **删除只带自身列级 CHECK 的列是合法的**：在 SQLite 3.46.1 实测通过（`libsqlite3-sys 0.30.1` 内置版本一致）；`ALTER TABLE ... DROP COLUMN` 会连同该列定义上的 CHECK 一起移除。
- **读取路径 fail-closed**：`nomifun-knowledge/src/service.rs:4414-4426` 的 `binding_from_row` 对任何不在允许列表内的存量值直接返回 `AppError::Internal`。它是唯一解码点，喂给挂载重建、会话打开、终端拉起、MCP 实时绑定解析与 broker。**所以先改允许列表后迁移数据 = 所有带知识库挂载的会话 500**。
- **历史事故**：`docs/handoffs/2026-07-31-terminal-kb-residuals-and-migration-collision.md` 记录两个分支各加了一个 `022_` 迁移，合并后一起发进 v0.3.5，**全体安装无法启动**。教训：新迁移文件版本号必须唯一（当前最高 `024`，本次用 `025`），且不得移除 `migration_file_versions_are_unique` 守卫测试。

## 1. 目标模型

```
挂载（knowledge_bindings）
├─ enabled: bool                 挂不挂
├─ writeback: bool               回不回写            ← 唯一开关，不变
├─ writeback_eagerness: 'manual' | 'auto'            ← 改值域，默认 manual
└─ channel_write_enabled: bool   外部渠道是否允许回写 ← 保留，语义从"强制暂存"改为"允许直写"
```

回写落点只有一种：直接写入知识库正文。代码级强制退化为二值：

```rust
pub enum WriteMode { Disabled, Direct }   // 原 { Disabled, Staged{scope}, Direct }
```

两种意识的行为差异落在**触发**上，而不只是措辞：

| | 回合末自动抽取 | 回合内 `knowledge_write` 工具 | provider 成本 |
|---|---|---|---|
| `manual` | **不跑** | 注册，用户明确要求时调用 | 0 |
| `auto` | 跑 | 注册，模型自主判断 | 每轮一次抽取调用 |

### 1.1 `resolve_write_policy` 改造后的完整落点

四个表面，不是三个 —— 外部渠道**仍然默认关闭**，只是开启后的落点从暂存变成直写：

```rust
let writeback = binding.enabled && binding.writeback;   // 不变
if !writeback { return Disabled }
match surface {
    Companion                  => Direct,               // 原本就是 Direct
    ExternalChannel            => if binding.channel_write_enabled { Direct } else { Disabled },
    RegularChat | TerminalAcp  => Direct,               // 原本读 writeback_mode
}
```

**这里最容易出的错**：现状的 `RegularChat | TerminalAcp` 分支是 `match writeback_mode { "direct" => Direct, _ => Staged }` —— 兜底是 **Staged**。把兜底机械改成 `Direct` 的人，会顺手把 `ExternalChannel` 也带成无条件直写，从而丢掉 `channel_write_enabled` 这道开关。该开关必须保留。

## 2. 后端改造清单（Rust）

### 2.1 `nomifun-knowledge`

| 位置 | 动作 |
|---|---|
| `context.rs:25-53` `WritebackMode` 枚举及 `parse` | 整段删除 |
| `context.rs:107-131` `KnowledgeContextOptions.writeback_mode` / `target_id` | 删除两个字段（`target_id` 全仓库只被 `:330` 的 `_inbox/{}/` 用一次） |
| `context.rs:281-346` `writeback_contract` | 四段（{工具,文件}×{暂存,直写}）压成两段（{工具,文件}×直写）；`mode` 参数删除 |
| `context.rs:348-366` `eagerness_clause` | 两个 `&'static str` 全文重写为手动型/自动型语义 |
| `service.rs:383-390` `WriteMode` | 删除 `Staged{scope}` 变体 |
| `service.rs:408-419` `StagedBaseSnapshot`、`StagedProposalMetadata` | 整体删除 |
| `service.rs:422-427` `WriteOutcome.staged` | 删除字段 |
| `service.rs:507-536` `resolve_write_policy` | 四个表面的落点见 §1.1；`scope` 参数失去唯一用途，一并清理 |
| `service.rs:2039-2164` `write_resolved_document_under_target_lock` | 删除暂存分支（~175 行内嵌）；直写 Update 分支按 §4 改为合并+CAS |
| `service.rs:2346-2756` `finalize_turn_writeback_with_progress` | 删除暂存分支；保留直写合并逻辑 |
| `service.rs` 暂存/待审专属方法 | 15 个方法 + 6 个自由函数 + 5 个类型，约 604 行非测试代码 |
| `service.rs:142-144` `KnowledgeBaseInfo.pending_inbox` | 删除字段及两处计算（`:4298-4305` 行内、`:1919-1929` `count_pending_inbox`） |
| `service.rs:46-55` `WRITEBACK_MODES` / `WRITEBACK_EAGERNESS` | 前者删除；后者值域改 `["manual","auto"]` |
| `service.rs:247-253` `default_writeback_mode` / `default_writeback_eagerness` | 前者删除；后者返回 `"manual"` |
| `service.rs:231-245` `KnowledgeBinding` | 删除 `writeback_mode` 字段（这个结构体**就是** POST 的 wire 契约，见 §3.1） |
| `service.rs:4414-4426` `binding_from_row` | 删除 mode 校验分支；eagerness 校验对新值域生效 |
| `routes.rs:70-88`、`:581-684` | 7 条待审路由 + 4 个路由内 DTO 全删；`:579` 段标题改写（`list_consumers` 存活） |
| `routes.rs:14-17` | 导入清单摘掉 `InboxDiff`、`InboxEntry`、`InboxMergeResult` |
| `lib.rs:47` | 摘掉 `InboxDiff`、`InboxEntry`、`InboxMergeResult`、`KB_INBOX_REL_DIR` 四个再导出 |
| `turn_writeback.rs:17-30` `TURN_WRITEBACK_SYSTEM` | 删除 `_inbox` 一行；重写候选门槛措辞 |
| `turn_writeback.rs:53-64` | 两个 match 全文重写为手动型/自动型 |
| `autogen.rs:18,222-226,244` | 删除 `KB_INBOX_REL_DIR` 导入与 `_inbox/**` 排除过滤器及其注释 |
| `broker.rs:750` | 测试字面量 `writeback_mode: "staged"` 删除 |
| `mcp_server.rs:546` | 删除 wire JSON 键 `"staged"` |
| `mcp_server.rs:46,400-406` | 删除 `opaque_workpath_write_scope`，同时清掉随之失用的 `sha2`/`hex` 导入 |
| `export.rs:8-10` | 更新政策注释（打包本就无路径过滤，行为不变） |
| `mount.rs`、`workspace_binding.rs` | **不改**，见 §5 假阳性清单 |

### 2.2 跨 crate 连带

| 位置 | 动作 |
|---|---|
| `nomi-agent/knowledge_tools.rs:255-259` `WriteMode`（2 变体镜像枚举） | 删除 `Staged` 变体，与 `nomifun-knowledge` 的同名枚举**原子同改** |
| `nomi-agent/knowledge_tools.rs:277` `WriteReceipt.staged` | 删除字段 |
| `nomi-agent/knowledge_tools.rs:315-322` `KnowledgeWriteTool::new` | 入参 4→3；调用点 `manager/nomi/agent.rs:761` 同改 |
| `nomi-agent/knowledge_tools.rs:418-424`、`:476-480` | 工具 schema 的 `content` 描述与回执文案按 §4 重写 |
| `nomifun-ai-agent/knowledge_writeback.rs:11,24-48` | 删除 `TMode::Staged → WriteMode::Staged` 映射与 `staged` 透传 |
| `nomifun-ai-agent/factory/nomi.rs:303-306` | 删除 `.unwrap_or_else(\|\| "staged".to_owned())` 默认兜底 |
| `nomifun-ai-agent/factory/nomi.rs:311`、`api-types/agent_build_extra.rs:395` | `knowledge_writeback_mode` 字段删除；`knowledge_channel_write_enabled` 保留 |
| `nomifun-ai-agent/manager/nomi/agent.rs:378,393,414` | `knowledge_writeback_staged: bool` 是 `new` / `new_with_host_wiring` 的**第 12 个位置参数**，删除必须同步全部调用点（`factory/nomi.rs:778` 约 14 个位置参数），否则类型相同的相邻参数会静默错位 |
| `nomifun-conversation/service.rs:12481-12561` `build_turn_writeback_request` | 新增手动型早退门（`if eagerness == "manual" { return None }`），紧接 `:12522-12526` 的意识读取之后 |
| `nomifun-conversation/service.rs:4643-4666` | 删除 preset mode → binding mode 映射；eagerness 兜底改 `"manual"` |
| `nomifun-conversation/service.rs:6321-6344` | 删除基于持久化 `staged` 标记与 `_inbox/{scope}/` 前缀的重试去重逻辑 |
| `nomifun-conversation/stream_relay.rs:476` | 删除 wire 字段 `written[].staged`（唯一生产者） |
| `nomifun-conversation/stream_relay.rs:313-322` | `turn_writeback_status_label` 是对 `TurnWritebackStatus` 的穷尽 match，随枚举同改 |
| `nomifun-companion/routes.rs:699-718` | 与 `conversation/service.rs:4643-4666` 重复的同一段逻辑，同改 |
| `nomifun-gateway/caps_knowledge.rs:105,129-137` `SetBindingParams` | 删除 `writeback_mode` 字段。注意该结构体是 `#[serde(deny_unknown_fields)]`，旧调用方发送该键将被硬拒 —— 属可接受的 wire 破坏，由契约版本号承担（§6） |
| `nomifun-gateway/caps_knowledge.rs:370` | `KnowledgeBinding{..Default::default()}` 的 `Default` 原先给出 staged，改后给出直写；需显式确认这是期望行为 |
| `nomifun-gateway/caps_knowledge_ext.rs:393-419` | 3 个待审 MCP 能力删除 |
| `nomifun-preset/service.rs:566-570` | eagerness 校验文案与值域改 `manual｜auto` |
| `nomifun-terminal/service.rs:4224,4258,4497,4553` | 构造/断言中的 mode 字段随之清理；`:4514-4521` 那条经由 `writeback_mode` 证明"读-改-写保留"的断言需换用其它存活字段重写，不得直接删掉 |
| `Cargo.toml` | 删除待审 diff（`nomifun-knowledge/service.rs:7310` 的 `similar::TextDiff::from_lines`）后，`similar` 在全仓库**再无使用者**（已核实：其余命中全是注释与函数名里的英文单词 similar）。同时摘除 `crates/backend/nomifun-knowledge/Cargo.toml:26` 与工作区 `Cargo.toml:132` |

## 3. 网关 / IPC / UI

### 3.1 wire 契约

`POST /api/knowledge/binding/{kind}/{target_id}` 没有独立请求 DTO —— `routes.rs:759-769` 直接把 body 反序列化成 `service::KnowledgeBinding`，所以**那个结构体就是 wire 契约**，删字段即破坏性变更。路由本身无 `deny_unknown_fields`，残留的 `writeback_mode` 键会被静默忽略；但 `'conservative'` 会命中 `set_binding` 的值域校验（`service.rs:3520-3530`）返回 400。因此所有仍在发送旧字面量的调用方必须**同一提交内**改掉：

- `ui/.../OtherTab/bundleIo.ts:99-108`（伙伴 bundle 导入，现发 `'staged'` + `'conservative'`）
- `ui/.../guid/hooks/useGuidAdvancedConfig.ts:70-71`（引导向导用 `!== 'staged' || !== 'conservative'` 判断是否需要持久化挂载）

删除的 HTTP 路由（7 条）：

```
GET  /api/knowledge/bases/{id}/inbox
GET  /api/knowledge/bases/{id}/inbox/diff
POST /api/knowledge/bases/{id}/inbox/merge
POST /api/knowledge/bases/{id}/inbox/discard
GET  /api/knowledge/inbox/pending-count
POST /api/knowledge/inbox/merge-all
POST /api/knowledge/inbox/discard-all
```

### 3.2 UI

| 位置 | 动作 |
|---|---|
| `pages/knowledge/InboxReviewPanel.tsx` | 整文件删除（275 行） |
| `pages/knowledge/useKnowledge.ts` | 删除 `useKnowledgeInbox`、`useKnowledgeInboxPending` |
| `pages/knowledge/KnowledgeDetailPage/index.tsx:1391-1408` | 删除待审 TabPane + Badge；连带清理 `:24` `Badge`、`:73`、`:77` 三个随之失用的导入（`noUnusedLocals` 会报错） |
| `pages/knowledge/KnowledgeCard.tsx` | 删除待审角标 |
| `components/layout/Sider/index.tsx:15,67,235` | 删除红点：导入、调用、`dot={pendingInboxCount > 0}` |
| `pages/conversation/components/KnowledgeControl.tsx:39-40,398-399,600,618,622` | **唯一**的模式/意识选择器：删除模式选择项，意识两项改文案与值。该组件由 4 个宿主表面渲染，改动会同时影响它们 |
| `pages/settings/PresetSettings/PresetEditDrawer.tsx:657` | 删除 `<Select.Option value='staged'>` |
| `pages/knowledge/KnowledgeDetailPage`「使用规则」第 3 步 | 现在枚举"关闭 / 暂存审阅 / 直接写入"，改为二元说明 |
| `common/adapter/ipcBridge.ts:5427-5428,5476,5483,5508-5522,5929-5993` | 删除 `pending_inbox`、`KnowledgeWritebackMode` 类型、`IKnowledgeInboxEntry`/`IKnowledgeInboxDiff`、7 个方法 |
| `common/chat/chatLib.ts:112-122,452-463` | 状态字面量联合与运行时 Set 白名单同步收缩 |
| `pages/conversation/Messages/components/MessageText.tsx:62-65` | 第三处状态字面量副本，三处必须同步 |
| `components/media/Diff2Html.tsx` | **保留** —— 与 `Workspace/components/FileChangeList.tsx:417` 共用 |
| `knowledgeConsumersUnmount.test.ts:13-14,25-26,36-37` | `toEqual` 硬断言含 `writeback_mode: 'direct'` / `writeback_eagerness: 'aggressive'`，随类型同改 |

### 3.3 i18n

- 删除待审键：`knowledge.detail.inbox.*`、`knowledge.inbox.*`、`knowledge.detail.inboxEmpty`（注意 `inboxEmpty` 是 `detail.inbox` 的**兄弟**而非子节点，只删对象会留下它）。
- 改写意识键：`knowledge.control.eagerness{Conservative,Aggressive}` → 手动/自动。
- 删除模式键：`knowledge.control.writebackMode` 及其选项。
- **所有 locale 必须同步改**：`localeKeyParity.ts:122-130` 把单侧存在视为硬错误。
- **必须跑 `bun run gen:i18n`**：`i18n-keys.d.ts` 由 `scripts/generate-i18n-types.mjs:337-340` 仅从 `en-US` 生成，手改或漏跑会让 `bun run check:i18n` 失败。
- 顺带清理三个**本就无调用点**的孤儿键：`knowledge.inbox.empty`、`knowledge.inbox.scopeLabel`、`knowledge.inbox.selectProposal`。

### 3.4 文档

| 位置 | 动作 |
|---|---|
| `README.md:167` ↔ `README.zh-CN.md:166` | "安全回写"卖点现在宣称"默认暂存到审阅收件箱 + unified-diff 预览 + 合并/丢弃"，全部不再成立，改写为手动/自动二元 |
| `docs/guides/companions.md:151-155` ↔ `companions.zh.md:64-66` | 唯一记录两种模式与 `_inbox/` 的地方，改写 |
| `CHANGELOG.md:6` `## Unreleased` 之后 | 新增条目；单语言文件，无 `CHANGELOG.zh-CN.md` |
| `ui-api-contract-version.txt` | `14` → `15`，随后必须跑 `bun run build:ui` |
| 两份 handoff、已发布 CHANGELOG 段落、`STATUS.md` | **不改**。前两者是受 `STATUS.md:73-78` 保护的历史记录；`STATUS.md` 本身零命中 |

## 4. 顺带修复：工具写入路径统一为合并 + CAS

**问题**（已核实）：`knowledge_write` 工具的直写 Update 分支（`service.rs:2154-2162`）调用 `write_file_under_target_lock` → `write_text_atomic`，**无条件重命名覆盖**。原文档从不被读取，因此无合并、无期望内容可比对；`validate_write_request`（`:6274-6289`）只查非空内容，无大小上限；`manager/nomi/agent.rs:573-576` 把该工具加入 allow_list **绕过审批门**。

而回合末回写的直写 Update 分支（`service.rs:2585-2653`）**已经**在做正确的事：读取现有 → `merge_direct_turn_writeback`（追加式合并，`:5817`）→ `write_file_if_unchanged`（CAS，`:1583`）。

**改法**：把 `:2154-2162` 换成 `:2588-2628` 已在跑的同一组调用。约 20 行服务层 + 约 10 行文案。

**连带**：

- `knowledge_writeback_e2e.rs:117-121` 现在断言的正是"直写 Update 覆盖原文"，需改为断言合并语义。
- `context.rs:312-318`（工具直写契约）当前教模型"read it first, merge, then write the full content"，却**漏掉**了文件路径契约在 `:336-337` 带的那句"Never rewrite documents wholesale"；改后由代码保证，文案同步。
- 自动型的"不重复记录"由 `contains_markdown_block`（`:5837`）的块级去重免费获得。

## 5. 假阳性防护清单（一次全局替换就会毁掉的无关功能）

| 位置 | 它其实是什么 |
|---|---|
| `nomifun-knowledge/workspace_binding.rs:58,224,548` | "conservative" 描述**锁保守性**，与回写意识无关 |
| `nomifun-knowledge/mount.rs:195,204,260-266,697` | `staging` / `StagedEntryCleanup` / `.nomi-managed-*` 临时文件是**原子替换机制**，与回写暂存无关 |
| `locales/*/nomi.json:223-226` | `preferenceConservative/Aggressive` 是**技能生成偏好**，对应真实的 `EvolvePreference` 枚举 |
| `locales/*/idmm.json:42-46` | `tendency.{conservative,balanced,aggressive}` 是**备用模型倾向**，三值 |
| `i18n-keys.d.ts:2064-2077`、`:3973` | `messages.knowledgeWriteback.*`、`settings.presetKnowledgeWriteback` 属于**存活**的回写 |
| `docs/guides/companions.md:106`、`.zh.md:44`、`CHANGELOG.md:263,279` | 保守/激进指 `EvolvePreference` |
| `docs/architecture/agent-execution.zh.md:126-127,162` | "回写"指 AgentExecution 的 Attempt 回写，与知识库无关 |

## 6. 迁移设计：`025_knowledge_writeback_simplification.sql`

`knowledge_bindings`（模板：`020_channel_owner_domain.sql:14-27`）：

```sql
ALTER TABLE knowledge_bindings ADD COLUMN writeback_eagerness_v2 TEXT NOT NULL
    DEFAULT 'manual' CHECK (writeback_eagerness_v2 IN ('manual', 'auto'));
UPDATE knowledge_bindings SET writeback_eagerness_v2 =
    CASE writeback_eagerness WHEN 'aggressive' THEN 'auto' ELSE 'manual' END;
ALTER TABLE knowledge_bindings DROP COLUMN writeback_eagerness;
ALTER TABLE knowledge_bindings RENAME COLUMN writeback_eagerness_v2 TO writeback_eagerness;
ALTER TABLE knowledge_bindings DROP COLUMN writeback_mode;
```

`preset_knowledge_policy`（baseline:1544-1546）同法处理 `eagerness`（它有独立 CHECK），并 `DROP COLUMN mode`。

要点：

- **上述配方已在 SQLite 3.46.1 端到端实测通过**（与 `libsqlite3-sys 0.30.1` 内置版本一致）：最终 schema 正确、CHECK 在 `RENAME COLUMN` 后仍然生效、部分唯一索引存活、`('staged','conservative')`→`manual` 与 `('direct','aggressive')`→`auto` 映射正确、不点名该列的 INSERT 拿到新默认值 `manual`、写入旧值 `'conservative'` 被 CHECK 拒绝。
- **CHECK 与 DEFAULT 必须同时改**：`sqlite_conversation.rs:6304-6314` 的 INSERT 不点名这些列，依赖 DDL 默认值；只收窄 CHECK 而留 `DEFAULT 'conservative'` 会让这些 INSERT 全部 CHECK 违例。
- **`RENAME COLUMN` 会自动更新 CHECK 里的列名引用**（SQLite ≥3.25）。
- **不做值域兼容映射**：`WritebackEagerness::parse` 不新增接受 `conservative`/`aggressive` 的分支。仓库标准（`docs/contributing/data-and-identifier-standards.md:49-53`）明确禁止"为回避决策而加兼容映射"；迁移已负责存量行，未知值仍由 `parse` 落到默认值并告警。
- **不改 baseline**，不动 `_sqlx_migrations` 校验和链。
- **备份包**：迁移一落地，旧备份包在新版本下即不可恢复 —— 但这不是本次引入的：`backup_bundle.rs:846` → `validate_current_migration_lineage`（`database.rs:191-198`）只接受 `Current`，任何新迁移都有同样效果。属于本仓库既有的、对每个迁移一致的行为。
- **表重建守卫**：本方案**不重建** `knowledge_bindings`。若后续有人改成重建，必须逐字复现 `id INTEGER PRIMARY KEY AUTOINCREMENT`、三处 UUIDv7 GLOB CHECK 与 4 个部分唯一索引 —— `id_schema_contract.rs:233-256` 在**每次打开库**时校验索引的规范化 WHERE 谓词文本。

## 7. 破坏性变更与用户可见影响

1. **待审里未处理的提案不再有界面入口**。文件仍在 `_inbox/` 原地，且因为本次删掉了 `_inbox` 排除过滤器，它们从此在文档树、`knowledge_search`、TOC、AI 概览采样与导出里**正常可见可搜** —— 从"隐形待审"变成"普通文档"。删除库时的警告文案如提到待审需同步改。
2. **存量 `conservative` 挂载 → `manual`**，回合末自动回写从此不再触发（原先会触发）。存量 `aggressive` → `auto`，行为最接近原状。这是有意的行为变更：新默认是更克制的一侧。
3. **存量 `staged` 挂载 → 直接写入库正文**，包括开了 `channel_write_enabled` 的无人值守渠道机器人。由 §4 的合并+CAS 承担安全性。
4. **旧 UI 包 / 外部 MCP 调用方**发送 `writeback_mode` 或旧意识字面量会拿到 400；由 `ui-api-contract-version.txt` 14→15 的启动期校验兜住 UI 侧。
5. **磁盘上残留的终端 README**（`{cwd}/.nomi/knowledge/README.md`）在会话重新拉起前仍写着暂存契约；这是既有的重建时机问题，不在本次修复范围，但需在计划里记明。

## 8. 测试策略

删除侧：`nomifun-knowledge` 约 1100 行暂存/待审测试删除。其中三处**不能直接删**，要改写以保住它们真正在保护的不变量：

- `nomi-agent/knowledge_tools.rs:743-752` `write_by_handle_builds_handle_target` —— 全仓库唯一验证"模型给的 `handle` 变成 `WriteTarget::Handle`"的覆盖，改成直写版本。
- `nomifun-terminal/service.rs:4514-4521` —— 经由 `writeback_mode` 证明"读-改-写保留其它字段"，换存活字段重写。
- `nomifun-ai-agent/factory/nomi.rs:2472-2492` —— `channel_write_enabled` opt-in 存活性回归，与 mode 断言解耦后保留。

新增侧：

- 迁移测试：造一个含 `('staged','conservative')` 与 `('direct','aggressive')` 行的库，跑迁移，断言映射结果、新 CHECK 生效、旧值被拒、不点名列的 INSERT 仍成功。
- 手动型早退门：`build_turn_writeback_request` 在 `manual` 下返回 `None`，且**不发生 provider 调用**。
- 自动型仍触发，且提示词含新措辞、不含 `_inbox`。
- §4 合并+CAS：工具直写 Update 不再截断原文；并发写入命中 CAS 冲突。
- `resolve_write_policy` 四个表面的落点断言。

## 9. 验收门与命令

```bash
cargo check --workspace
cargo test -p nomifun-db -p nomifun-knowledge -p nomifun-conversation \
           -p nomifun-ai-agent -p nomi-agent -p nomifun-preset -p nomifun-gateway -p nomifun-terminal
bun run gen:i18n          # 必须在改 locale 之后
bun run check             # 含 check:i18n / check-agent-vocabulary / check-dead-css-utilities
bun run test:ui           # bun run check 不含它，而受影响测试在这里
bun run build:ui          # 契约版本号改动后必须
```

`bun run check` **不跑任何 Rust 步骤**，也不跑 `test:ui` —— 两者必须单独执行。

## 10. 明确不做（YAGNI）

- 不为回写加文档历史 / 快照 / 回滚 UI（§0.2 已在三个选项中选了成本最低且够用的一档）。
- 不新增回写频率计数与配额。
- 不新增用户意图分类器、关键词前置过滤或 `/记一下` 斜杠命令：用户原话在回合内已同时出现在模型上下文与抽取提示词中，手动型靠"注册工具 + 提示词"即可，无需新信号。
- 不把回写意识提升到知识库级。
- 不拓宽现有的"重试回写"入口为通用的按需回写手势（可作为后续独立议题）。
- 不重建 `knowledge_bindings` 表。
- 不修 `docs/architecture/backend-crates.md:95` 等**先前就已过时**的描述（"scoped read-only knowledge MCP server"），除非它落在改动路径上。
