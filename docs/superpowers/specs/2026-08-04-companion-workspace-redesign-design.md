# 桌面伙伴管理页重构设计 / Companion Workspace Redesign

- 日期：2026-08-04
- 状态：已定稿，实施中
- 影响面：`ui/src/renderer/pages/nomi/**`、`crates/backend/nomifun-companion/**`、`crates/backend/nomifun-gateway/src/caps_*.rs`、`ui/src/common/adapter/ipcBridge.ts`、i18n 两语、`docs/guides/companions*.md`

## 1. 问题陈述

现有 `/nomi` 管理页是全应用视觉与信息架构最弱的一处：

1. **没有层次。** 顶层是一个 Arco `Radio.Group` 的三态"域"切换（伙伴 / 共享 / 形象库），第二层又是一个 `Radio.Group` 的七个 tab。两层同构控件叠放，用户无法从形状上判断哪层是什么。
2. **没有边界。** `OverviewTab` 是大杂烩：两条 Alert（自学习披露、采集未开启）+ 基础配置 + 等级卡 + 形象尺寸滑块 + 周报。`SettingsTab` 又重复渲染了一次 `CharacterPicker`（总览里是弹窗，设置里是内联），并且混入人格、静音时段、删除伙伴三种完全不同性质的操作。
3. **计数器在骗人。** 总览的"记忆 / 新建议 / 专精技能"三个数字全部来自跨伙伴的全局计数（`count_memories` 无 owner 谓词），却渲染在单个伙伴的卡片里。
4. **"共享"域是伪抽象。** 域切换本身是装饰；实质是 `SharedCompanionConfig` 这个单例把学习、进化、采集、归档全部做成了装机级全局配置，于是"这个设置属于谁"永远说不清。
5. **视觉与应用其余部分脱节。** `height: calc(100vh - 196px)` 硬算高度、`max-w-[95%]` 百分比列宽、`bg-fill-1 rd-12px` 自制侧栏、混用两套图标库——而 `pages/modelHub` 与 `pages/requirements` 早已把"左侧 `ContentSider` + 中间内容窗格"这套 IA 做成了成熟范式。

目标：按用户白板重画的三栏布局重构，删除四项历史功能及其全部代码与数据，不留占位式死控件。

## 2. 目标与非目标

**目标**

- 三栏：左侧伙伴列表栏（新建 / 列表 / 位置调整 / 形象库入口）、中间七标签工作区、右侧按需展开详情面板。
- 七个标签：总览 / 记忆&知识库 / 远程控制 / 进化 / 技能 / 聊天历史 / 其他。
- 彻底删除：建议、共享域、共享记忆、技能专精。含代码、路由、DB、i18n、文档、测试。
- 视觉上与 `modelHub` / `requirements` / `knowledge` 同源，复用既有 design system，不发明新 token。

**非目标（本次不做，理由见 §7）**

- VAD、per-companion ASR、视觉大模型选择、TTS 模型、声音音色、物联网连接。
- `companion_memories` 作用域列的物理删除（表重建）。
- 事件的按伙伴归属（tool call 事件天然没有伙伴归属）。

## 3. 信息架构

### 3.1 三栏骨架

```
┌ ContentSider (248px, 可拖拽 200–360, 持久化) ┬ 中间工作区 ─────────┬ 右侧详情 (按需) ┐
│ ┌ header (sticky, 不滚动) ─────────────┐   │ 伙伴身份头            │ 圆角浮起卡片      │
│ │ [+ 新建伙伴]  柔和 primary CTA       │   │ SegmentedTabs ×7      │ 左缘拖拽把手      │
│ └──────────────────────────────────────┘   ├───────────────────────┤ 宽度持久化        │
│ 伙伴行 ×N（可拖拽排序）                    │ NomiScrollArea        │ ?pane= 深链       │
│  头像 + 名字 + Lv + 模型就绪点             │  max-w-1100px 居中    │                   │
│  hover 显示 拖拽把手 / 删除                │  padX 由窗格宽决定    │                   │
│ ┌ footer (sticky) ─────────────────────┐   │                       │                   │
│ │ 🖼 形象库                             │   │                       │                   │
│ └──────────────────────────────────────┘   │                       │                   │
└────────────────────────────────────────────┴───────────────────────┴───────────────────┘
```

- 左栏用 `components/layout/ContentSider`（需新增 `footer` slot），行语法沿用全应用规范：`h-34px`（带头像 44px）`rd-8px gap-8px px-10px`，选中 `!bg-primary-1 !text-primary-6`，空闲 `hover:bg-fill-2 active:bg-fill-3`。
- 宽度用 `hooks/ui/useResizableSplit`，`storageKey: 'nomifun:nomi-sider-width'`。
- 中栏横向内边距用 `hooks/ui/useContainerWidth` 按**窗格宽**而非视口宽决定（三栏下视口断点会失真）。
- 标签条用 `components/base/SegmentedTabs`（`size='sm'`），替换掉 `Radio.Group`。新增**圆点徽标** slot（只有点，不显示数字）用于引导注意力。
- 右栏参照 `pages/conversation/components/ChatLayout` 的按需右窗格，但需抽成可复用的 `ContentAside`。
- 移动端：不渲染左栏，改为伙伴 `Select` + `SegmentedTabs`。

### 3.2 标签内容归属

| 标签 | 内容 | 右侧详情面板触发 |
|---|---|---|
| **总览** | 伙伴形象（名字 / 桌面形象 / 成长等级）· 伙伴设定（角色介绍 / 设定复用）· 模型配置（主 / 小 / 备用对话模型；语音与感知状态行→深链 /models） | 形象选择器、设定复用预设选择 |
| **记忆&知识库** | 记忆（检索 / 添加 / 编辑 / 删除 / 批量 / 合并 / 待确认队列 / 从其他伙伴导入）· 知识库绑定 · 外置记忆（预留，无控件） | 记忆详情与编辑、合并助手、跨伙伴导入选择器 |
| **远程控制** | IM channel 连接（沿用 `RemoteConnectSection`）· 远程访问令牌（从应用侧栏迁入） | 平台配置 |
| **进化** | 学习配置（开启 / 周期 / 模型 / 数据范围）· 技能生成配置（开启 / 保守·激进偏好）· 休眠时段 | — |
| **技能** | 统一技能列表：已配置能力（catalog）+ 自动生成技能（草稿→启用→归档），SKILL.md 编辑、从会话学习 | 技能详情 / SKILL.md 编辑器 |
| **聊天历史** | 按天列表 + 当日原始逐条阅读器；当天有归档摘要则置顶摘要卡；「去年今日」筛选 | 当日阅读器 |
| **其他** | 迁移（导出范围可选：记忆默认 / 技能可选 / 设定默认；导入）· 删除伙伴 | — |

### 3.3 URL 状态

沿用现有 `?companion=<id>&tab=<key>`，新增 `?view=figures`（形象库接管中栏，左栏保留）与 `?pane=<kind>&paneId=<id>`（右侧面板）。全部 `replace: true`，与本页既有约定一致。切换伙伴时关闭右侧面板。

删除三条历史深链兼容 shim（`modelKnowledge→knowledge`、`learn→collect`、`chat→` 重定向）。

## 4. 删除清单

### 4.1 建议 / Suggestions（整体删除）

- 前端：`tabs/SuggestionsTab.tsx` + 测试；`ICompanionSuggestion*` 类型与 `listSuggestions` / `decideSuggestion` bridge；`fromApiCompanionSuggestion`；`useNomi.ts` 的两处 WS 订阅；`ICompanionStatus.suggestions_new` 与总览计数。
- 后端：`GET /api/companion/suggestions`、`POST /api/companion/suggestions/{id}/decide` 及 handler；`companion_suggestions` 表与索引（`DROP TABLE`，沿用 `companion_learn_runs` 退役先例）；`prompt.rs` LEARN_SYSTEM 的建议规则 4/10 与 `LearnedSuggestion`；`learner.rs` 建议持久化；WS `companion.suggestion-created` / `-decided`；agent 工具 `nomi_companion_list_suggestions` / `nomi_companion_decide_suggestion`。
- 桌宠窗口的未读徽标与详情弹窗中渲染建议卡片的部分。
- i18n：`nomi.suggestions.*`（9 键）、`nomi.tabs.suggestions`、`nomi.overview.newSuggestions`。

**替代通道（实施时修正）。** 原计划把 `propose_companion_memory`（召唤会话写回）改用 `companion_memories` 的 `pending` 状态在「记忆&知识库」里复核。实施中确认：建议卡片是该能力**唯一的存储与唯一的复核界面**，所以它随建议一起删除了，而不是改道。这是有意的取舍——保留一个没有确认界面的写回工具，等于留一个静默写用户记忆的后门。若之后仍需要这个能力，它需要一份新的"写前确认"设计，而不是恢复本次删除的一部分。

### 4.2 共享域 / Shared domain

- 前端：`SHARED_TABS`、`Domain` 类型、`setDomain`、三态域 `Radio.Group`、`useCompanionShared` 及 `mergeSharedConfig`；`nomi.domains.*`。
- 后端：`SharedCompanionConfig` 中 `learn` / `evolve` 的字段迁至 `CompanionProfileConfig`（§5.1）；`smart_collaboration` 与 `bridge_to_memory_dir` 删除（前者属"共享"设计，后者无 UI、无 TS 类型、仅 agent 工具可写）；`collect` / `archive` 保留为装机级但 UI 迁出本页（§5.2）；`default_companion_id` 保留（记忆归属解析需要它）。
- `PATCH /api/companion/config` 收窄为仅 `collect` / `archive` / `default_companion_id`。

### 4.3 共享记忆 / Shared memory

**分级执行——这是本次唯一涉及用户既有数据的改动。**

本次交付（Stage 1–3，无表重建）：

1. **数据安全前置**：`find_similar_active` 加 owner 谓词；`import_memory_bundle` 把不在本机名册里的 owner 就地改写为默认伙伴（`validate_companion_references` 在启动时对孤儿行硬失败，跨机导入会导致下次启动即砖）；`CompanionMemory` 的两个作用域字段加 `#[serde(default)]` 容忍旧 bundle（`deny_unknown_fields` 会让已导出的 .zip 全部无法再导入）；补一条"共享记忆确实进入任意伙伴 prompt"的回归测试（今天完全无守护）。
2. **写入方改归属**：`learner.rs:261`、`caps_memory.rs:109`、`companion.rs:1143`、`service.rs:1562` 全部解析出具体 owner（`default_companion_id` → 否则名册最早一位）。移除 `add_memory` 的 `if scope == Shared` 去重闸门。重写 `prompt.rs:56` LEARN_SYSTEM（今天字面告诉模型它管理"这台电脑上所有电子伙伴共享的记忆中枢"）。
3. **一次性回填**：`upgrade_schema_in_place` 中幂等执行 `UPDATE companion_memories SET scope_kind='companion', scope_companion_id=<owner> WHERE scope_kind='user'`。**改写归属，不复制、不删除**——复制会按名册规模放大行数、破坏 `memory_id` 稳定性，且每份副本独立衰减归档，同一事实会静默分叉。
4. **移除 wire 与 UI**：`scope_from_parts`、三个路由请求形状的作用域字段、ipcBridge 的作用域字段与参数、`MemoriesTab` 的全部作用域 UI（20 处）、`MemoryScopeFilter` 与 `FilterClause.scope_owner`（已无生产调用方）、6 个 `nomi.memories.scope*` i18n 键。`ui-api-contract-version.txt` 从 4 递增。

延后（Stage 4，见 §7）：物理删除 `scope_kind` 列的表重建。

**名册为空时**：保留 `('user', NULL)` 在 DB 层合法（仅从 UI/wire 移除），回填幂等、有伙伴后再次运行即生效。这样"可以删到零个伙伴"这项既有承诺不被打破，且零重建风险。

**必须改的文案**：`nomi.settings.deleteCompanionHint` 与 `deleteConfirmBody` 今天承诺"共享记忆不受影响"，改归属后删除伙伴会连带删除其全部记忆——两条文案重写，删除确认里增加"先导出"提示。README 的头条卖点（`README.md:134` / `README.zh-CN.md:133` 的"One brain, many faces… share a common memory hub"）同步改写。

### 4.4 技能专精 / Skill specialization

保留自进化挖掘引擎（`evolution/engine.rs`），删除"专精"这套框架：

- 删除 `POST .../skills/{id}/gift`（跨伙伴赠送——属于被删的"共享"设计）及其 bridge、UI、i18n。
- 删除 `companion_skills.scope_companion_id IS NULL`（共享技能）这一路径与 `include_shared` 查询参数、两个 partial 索引。
- 删除全部"专精"措辞与计数：`nomi.skills.learnedTitle`（伙伴专精）、`nomi.overview.skillsActive`（专精技能）、`ICompanionStatus.skills_active`、周报的专精计数。
- 保留：挖掘、草稿→启用→归档生命周期、SKILL.md 编辑、从会话学习（示范教学，仅改称"从会话学习"）。生成技能与 catalog 能力合并进**同一个**技能列表，不再是两张分离的列表。

### 4.5 顺带清理的死代码

- `tabs/CollectTab.tsx` 迁出本页（→ 应用设置 · 隐私与数据采集）。
- `index.structure.test.ts`（266 行硬编码当前 IA 的字符串断言）删除重写。
- `uno.config.ts:51-55` 的 `borderColors` 块：`border-b-*` 被 UnoCSS 解析为 border-bottom-color，这五个键不可达。连同 `MIGRATION.md` 里错误的指引一并修正。
- `border-border-2` / `border-border-3`（36 处，15 文件）**不产生任何 CSS**——本页范围内全部换成 `border border-solid border-[var(--color-border-2)]`，并记录为全仓清理待办。
- `pages/nomi/*` 中 `@arco-design/web-react/icon` 的引用统一换成 `@icon-park/react`（应用其余新页面的规范）。
- `ReviewTab.tsx` 并入聊天历史后删除。
- `CompanionSessionRail` 更名 `CompanionSidebar`——"会话切换栏"是聊天迁出前的遗留命名。

## 5. 后端改动

### 5.1 进化配置改为按伙伴

`CompanionProfileConfig` 新增：

```rust
pub struct CompanionLearnConfig {          // 原 SharedLearnConfig
    pub enabled: bool,                      // 默认继承迁移前的全局值
    pub interval_minutes: u32,              // 5..=1440
    pub model: Option<ProviderWithModel>,   // 空 = 用伙伴的小对话模型，再空 = 主对话模型
    pub sources: LearnSources,              // tool_calls / chat_user_messages / requirements
}
pub struct CompanionEvolveConfig {          // 原 SharedEvolveConfig
    pub enabled: bool,
    pub preference: EvolvePreference,        // Conservative | Aggressive
    pub min_distinct_sessions: u32,
}
pub enum EvolvePreference { Conservative, Aggressive }
```

- `保守` = 必须用户要求（`auto_activate=false`）；`激进` = 自动评估高置信则生成（`auto_activate=true`, `auto_threshold=0.85`）。两个模式映射到既有的 `auto_activate` + `auto_threshold`，不再暴露裸阈值。
- 迁移：把迁移前的 `SharedCompanionConfig.learn` / `.evolve` 值复制给**每一个**既有伙伴作为初值。纯增量、幂等。
- 修掉既有 wire drift：`ICompanionEvolveConfig` 缺 Rust 侧的 `skill_half_life_days` / `skill_archive_threshold`。
- 归档器今天静默借用 `learn.model`（`archiver.rs:154`）——改为显式使用伙伴的小对话模型，回落主模型。
- 学习循环改为按伙伴调度，共享事件池 + 每伙伴游标。事件本身不做伙伴归属（tool call 事件天然无归属，补齐需要全新链路且不可靠）。
- `休眠时段`（`appearance.quiet_start/end`，今天只有渲染进程的桌宠窗口读它）新增服务端语义：休眠时段内跳过该伙伴的学习与进化调度。不影响 IM 自动回复（那会造成意外静默）。

### 5.2 采集配置迁出

`CollectConfig`（记录哪些事件源）本质是"这台机器记录什么"的隐私设置，不是伙伴属性。后端保持装机级不变，UI 从 `tabs/CollectTab.tsx` 迁至应用设置的隐私分区。伙伴侧只保留"从已记录的源中，这个伙伴学习哪些"（`CompanionLearnConfig.sources`）。两级模型，边界清晰。

### 5.3 伙伴排序

`CompanionProfileConfig` 新增 `order_index: Option<i64>`，进 `ICompanionProfilePatch`。`registry.list()` 排序改为 `order_index NULLS LAST, created_at, companion_id`。UI 用 `@dnd-kit`（仓库已有依赖与两处先例）实现拖拽。注册表内部的 `seq`（#N 显示号）从 UI 撤下——它是永不复用的水位号，与用户排序天然矛盾。

### 5.4 聊天历史按天

新增 `GET /api/companion/companions/{id}/history/days` → `[{ day: "YYYY-MM-DD", message_count, has_digest }]`，按本地时区分组该伙伴唯一会话的消息。逐日读取复用既有 `GET /api/conversations/{id}/messages`，新增 `day` 参数。当日若有 `companion_session_windows` 摘要则一并返回。

### 5.5 导出范围

`export_companion_bundle` 今天只写 4 个条目（manifest / companion.json / state.json / knowledge_refs.json），`file_count` 硬编码 `3u64`、`memories: 0`——即"导出伙伴"实际不含记忆也不含技能。补齐：`memories.jsonl`（默认）、`skills/`（可选）、`figure`（形象图）；请求体新增 `include_memories` / `include_skills` 范围标志；设定始终包含。

## 6. 视觉设计

复用既有 design system，**不新增任何 CSS 变量**（`check:theme` 要求新 token 在 5 个预设主题的亮暗两块中对称声明）。

- 选中态一律 `!bg-primary-1 !text-primary-6`（`--primary-1` 在暗色主题会反转为深色调，这是全应用唯一在两种模式下都成立的选中色）。
- 分区容器一律 `NomiSettingSection` / `NomiSettingList` / `NomiSettingRow`；表单控件用 `NomiInput` / `NomiSelect` 的 `contentFit`（字段缩到内容宽而非拉满整行，这是现有伙伴设置行显得整齐的原因）。
- 卡片：`rounded-16px border-2 bg-2 p-18px`，hover `-translate-y-2px` + `0_14px_38px` 阴影（`KnowledgeCard` 规范）。
- 主 CTA 用 `KnowledgeListPage` 的柔和处理：`rounded-full px-18px py-9px font-700` + 12% primary 底 + 柔和投影，而非饱和的 Arco primary。这是应用里最好看的 CTA。
- 空状态用 `WorkspaceEmptyState` 规范：72px 圆形 `fill-2` 徽标 + 16px 标题 + 13px 三级描述 + 一个圆角主 CTA。
- 半透明陷阱：`--color-fill-*` 是 rgba，嵌套会叠加变浊；浮在文字上方的元素（hover 动作按钮、吸顶头）必须用不透明面（`--bg-2`/`--bg-1`，暗色 `--bg-5`/`--bg-6`），见 `layout.css:12-23`。
- 自定义可点区用 `<div role='button' tabIndex={0}>` 而非裸 `<button>`（WebView2 会给真 button 画黑色焦点框，仓库多处有记录）。
- 键盘可达：左栏 `role='tablist' aria-orientation='vertical'` + roving tabIndex + 方向键/Home/End，照抄 `modelHub/index.tsx`。

## 7. 明确延后项与理由

| 项 | 为何本次不做 |
|---|---|
| VAD、per-companion ASR、视觉模型选择、TTS 模型、声音音色、物联网 | 全部是未建的后端子系统。`crates/backend/nomifun-device` 不存在；这些在 `docs/superpowers/specs/2026-08-03-xiaozhi-robot-integration-design.md` 里已有获批设计。**渲染无功能的占位控件正是"信息元素多、眼花缭乱"的成因**，因此本次不放假控件；总览的"语音与感知"只放状态行并深链 `/models`。 |
| per-companion 对话语言 | 代码库有意拒绝按伙伴钉死回复语言（`companion.rs:123-128` 及守护测试 `companion_system_prompt_does_not_force_a_reply_language`）。反转这个设计需要独立决策。 |
| `scope_kind` 列的物理删除 | 需要 12 步表重建（SQLite 不允许 DROP 被 CHECK 引用的列），且 `validate_baseline_schema` 比对精确有序列清单——漏任一步则**所有既有装机启动失败**。零用户可见收益，风险最大。回填 + wire/UI 移除已达成产品目标，物理列成为恒为 `'companion'` 的残留列。 |
| 事件按伙伴归属 | tool call / 终端 / 需求事件天然没有伙伴归属，补齐需要全新链路且结果不可靠。 |

**降级不可逆**：`DROP TABLE companion_suggestions` 后回退到 0.3.8 会在启动时因精确表集校验硬失败。沿用 `companion_learn_runs` 退役先例，在 CHANGELOG 中明示。

## 8. 组件边界

新增/重写的前端单元，每个单一职责、可独立理解：

```
pages/nomi/
  index.tsx                    三栏骨架 + URL 状态编排（目标 <200 行）
  CompanionSidebar/            左栏：header CTA / 可排序列表 / footer 形象库入口
  workspace/
    WorkspaceHeader.tsx        伙伴身份头 + SegmentedTabs（含徽标）
    tabs/OverviewTab/          形象 · 设定 · 模型配置
    tabs/MemoryTab/            记忆列表 / 待确认队列 / 跨伙伴导入 / 知识库绑定
    tabs/RemoteTab/            IM channels + 访问令牌
    tabs/EvolutionTab/         学习 · 技能生成 · 休眠时段
    tabs/SkillsTab/            统一技能列表
    tabs/HistoryTab/           按天列表 + 逐日阅读器
    tabs/OtherTab/             迁移 + 删除伙伴
  aside/                       右侧详情面板宿主 + 各 kind 的内容
components/layout/ContentAside/  从 ChatLayout 抽出的可复用按需右窗格
components/layout/ContentSider/  新增 footer slot
```

`MemoriesTab.tsx` 目前 725 行、`SkillsTab.tsx` 626 行——拆分是本次的附带收益，也让后续编辑更可靠。

## 9. 测试策略

- 删除 `index.structure.test.ts`；新写的结构测试断言**行为契约**（标签键集合、URL 参数往返、删除确认存在 danger 状态），不断言 className 字面量。
- `figureActionsVisual.test.ts` 冻结了 `figure-library-card w-184px h-234px` 等几何——形象库若重排需同步更新（注意这些 class 在任何 CSS 里都不存在，只是测试锚点）。
- 后端必加：共享记忆确实进入任意伙伴 prompt 的回归测试（今天无守护）；回填迁移不丢记忆的测试（扩展 `legacy_v3_file_store_upgrades_in_place_preserving_memories`）；跨机导入 bundle 后能正常启动（孤儿 owner 被改写）的测试。
- 每个实施波次结束时 `bun run check` + `bun test --cwd ui` + `cargo nextest run` 全绿再提交。

## 10. 实施顺序

**Wave 1 — 纯增量后端使能（不删任何东西，永远可编译）**：按伙伴 learn/evolve 字段 + 迁移；`order_index`；按天历史接口；导出范围；记忆 `pending` 状态。

**Wave 2 — 前端重写（用户痛点在此，优先交付可见价值）**：三栏骨架、`ContentAside`、`ContentSider` footer、七个标签、删除旧前端文件（建议 tab、共享域、作用域 UI、专精 UI）。

**Wave 3 — 后端减法（此时已无调用方）**：建议子系统、共享配置字段收窄、记忆作用域 wire、技能赠送/共享。

**Wave 4 — 收尾**：i18n 两语 + 类型再生成、测试重写、文档（`docs/guides/companions*.md` 的共享记忆章节、README 头条卖点）、`ui-api-contract-version.txt` 递增、CHANGELOG。

Wave 1 全增量所以始终绿；Wave 2 交付可见成果；Wave 3 是"已无引用"指引下的纯删除。任一波次结束都是可编译、测试通过、应用可运行的状态。

## 11. 实施状态（2026-08-04）

分支 `feat/companion-workspace-redesign`，4 个提交，未推远端。

**已完成**

- 三栏骨架、`ContentAside`、`ContentSider` footer slot、`SegmentedTabs` 注意力圆点。
- 七个标签全部实现并各自拆分为聚焦文件（旧 `MemoriesTab` 单文件 725 行 → 新 `MemoryTab` 9 个文件，最大 279 行）。
- 侧栏：新建 CTA / 可拖拽排序名册 / 形象库底部入口；`order_index` 落到 profile 与 registry 排序。
- 建议功能整体删除（前端、桌宠未读徽标、分离弹窗及其 Rust 窗口模块、两个路由、两个 WS 事件、两个 agent 工具、learner 的建议蒸馏、`companion_suggestions` 表）。
- i18n 两语落地 167 个新键、按代码证据删除 144 个死键，键集与顺序逐字节对等。
- 删除死代码：`useCompanionShared`、`pages/nomi` 下三处 Arco 图标引用改为 icon-park。
- 新增 `workspace/shell.structure.test.ts`（标签注册表 / URL 契约 / 删除不变量，经变异测试确认有效）与 `shellPrimitives.test.ts`。
- 契约版本 4 → 5；CHANGELOG 记录降级不可逆。

**验证证据**

- `bun run check` 全 8 个子门通过；`bun test --cwd ui` 1661 通过 / 1 失败（该失败在干净树上同样存在，为测试间污染，非本次引入）；`cargo nextest run -p nomifun-companion` 236/236；`cargo check --workspace --all-targets` 干净；`bun run build:ui` 通过。
- **迁移端到端实测**：把真实 0.3.8 期数据目录（含 `companion_suggestions` 表）复制出来、植入一条金丝雀记忆，用真实 `nomifun-web` 启动 —— 后端正常启动（无启动即砖）、表已删除、金丝雀记忆逐字保留、external-content FTS5 索引仍能检索到它、`/api/companion/suggestions` 返回 404。
- **未做**：UI 的视觉验证。本机为 Wayland 会话且无截图工具，Firefox headless `--screenshot` 连本地静态页都无法产出，因此新页面的实际观感尚未被人眼或截图确认过——这是本次交付最大的未验证面。

**未完成（按优先级）**

1. 共享记忆改归属（§4.3 Stage 1–3）：写入方改归属、一次性回填、wire/UI 移除。当前 `MemoryDetailPane` 会为 `scope_kind='user'` 的行显示"装机级"只读标注，是诚实的过渡态，但共享记忆本身尚未删除。
2. 技能专精的后端残余：`giftSkill` 路由与共享作用域技能行仍存在（UI 已不再调用，`include_shared: false`）。
3. 进化配置改为按伙伴（§5.1）：目前 `EvolutionTab` 通过 `useEvolutionConfig.ts` 适配器读写装机级共享配置，并在每个分区标注"当前对所有伙伴生效"。该文件顶部注明了迁移接缝位置。
4. 采集配置迁出到应用设置 · 隐私（§5.2）。
5. 聊天历史的后端按天索引接口（§5.4）；当前为客户端分页分组 + 显式「加载更早」。
6. 导出范围补齐（§5.5）；当前范围复选框中后端无法兑现的项已禁用并标注。
7. 文档：`docs/guides/companions*.md` 与 README 头条卖点仍在描述"共享记忆中枢"，需随第 1 项一起改。

