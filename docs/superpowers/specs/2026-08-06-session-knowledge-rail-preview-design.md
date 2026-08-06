# 会话右栏知识库预览入口

- 日期：2026-08-06
- 状态：设计已定稿，待实施。分支 `feat/session-knowledge-rail-preview`，未推远端。
- 影响面：`ui/src/renderer/pages/conversation/Workspace/KnowledgePanel/**`（新增）、`ui/src/renderer/pages/conversation/components/ChatConversation.tsx`、`ui/src/renderer/pages/terminal/{TerminalSessionPage,TerminalWorkspaceRail}.tsx`、`ui/src/renderer/pages/knowledge/KnowledgeDetailPage/{index.tsx,treeModel.ts}`、i18n 两语。后端零改动。

## 1. 问题陈述

会话可以挂载知识库（`KnowledgeControl` 弹窗，`ui/src/renderer/pages/conversation/components/KnowledgeControl.tsx`），但挂载之后**用户无法在会话里看见挂了什么、里面有什么内容**。想确认知识库里那篇文档写了什么，只能离开会话、进 `/knowledge`、找到那个库、再进详情页翻文件树。

同时，会话右侧已经有一条工具 rail（`工作区文件` / `变更` / `会话终端` / `协作任务`），是"这个会话拥有哪些资源"的既有归位处。知识库属于同一类资源，却不在那里。

本次只做这一件事：**挂载了知识库的会话，在右侧 rail 上多一个知识库 icon，点开是一棵按知识库分根的只读文件树，点文件在既有预览列渲染内容。** 不新增任何业务逻辑，不动任何后端。

## 2. 目标与非目标

**目标**

- 会话挂载了知识库时，右侧 rail 显示知识库 icon；未挂载时不显示。
- 三类会话都支持：普通会话、终端会话、伙伴会话。
- 挂载了 N 个知识库就显示 N 个根目录。
- 点击文件渲染其内容。
- 一个「全部展开 / 全部折叠」开关：展开 = 每个根各展开一层；折叠 = 只剩根目录。

**非目标**

- **不做编辑。** 这是预览入口。不引入 `writeFile` / `deleteFile` / `createFolder` / `renameTreeEntry`，不做右键菜单。要改知识库内容，仍去 `/knowledge` 详情页。
- **不新增后端接口。** `getBinding` + `listBases` + `listTree` + `readFile` 已经够用（§5）。不改 `ui-api-contract-version.txt`（当前 `16`，只为 HTTP/WS 线型变更而动）。
- **不放宽 rail 的显示门槛。** 见 §3.2。
- **不重构 `KnowledgeDetailPage`。** 那棵树是 1573 行文件里的内联 JSX（`index.tsx:1151-1263`），无 props 接口，读 ~11 个页面 `useState` 与 8 个 handler，且有按源码文本断言的测试（`knowledgeDetailActionBar.test.ts`）。本次只上提一个纯函数（§6.1），不抽组件。
- **不修 `KnowledgeControl` 的写入路径。** 它存在一处既有的绑定行解析分歧（§4.3），本次只让新面板读对，弹窗照旧，记为独立后续。
- 不做搜索框。详情页有搜索是因为它是全库管理界面；rail 是 260px 的速查入口，先不加。

## 3. 挂载判定与显示条件

### 3.1 判定：`enabled && kb_ids.length > 0`

`IKnowledgeBinding`（`ui/src/common/adapter/ipcBridge.ts:5622-5641`）有一个总开关 `enabled`，与 `kb_ids` 相互独立。图标显示条件取**两者都成立**：

```ts
const mounted = Boolean(binding?.enabled && binding.kb_ids?.length);
```

理由：这与会话列表里那颗知识库能力点的判定逐字相同（`SessionList/hooks/useWorkpathKnowledge.ts:69`），用户在两处读到的是同一个信号；且 `enabled === false` 时智能体本身也读不到这些库，让用户在会话里翻它会产生"它在生效"的错觉。

### 3.2 显示门槛沿用现有 rail，不放宽

rail 整体由 `workspaceEnabled = Boolean(conversation.extra?.workspace)` 控制（`ChatConversation.tsx:498, 549, 666`），`ChatSlider.tsx:21-23` 在无工作区时也返回空 div。曾考虑放宽成 `workspaceEnabled || knowledgeMounted`，核查后**确认不需要**：

1. 后端在创建时无条件分配工作区。四个创建入口全部汇入 `create_inner`（`crates/backend/nomifun-conversation/src/service.rs:4031`）；未给自定义工作区时走无类型分支的 `allocate_temp_workspace_id`（`:4266-4274`），随后落盘 `extra.workspace`（`:4589-4590`）；失败则 `compensate_failed_creation` 删行（`:4667`），不留半成品。
2. 读路径二次兜底。`get`（`:4771`）与 `list`（`:4821`）都调 `rebase_managed_workspace_in_row`（`:12785-12801`），每次读取都从 `temp_workspace_id` 重算并回写 `extra.workspace`。
3. 挂载本身就需要工作区。`prepare_mounts_for_*`（`:12316-12322`）把库挂到 `{workspace}/.nomi/knowledge/` —— 没有工作区的会话不可能挂载成功。
4. 终端侧 `workspaceEnabled` 是硬编码 `true`（`TerminalSessionPage.tsx:95`）。

结论：门槛为假的情形只有「会话数据加载中」（此时整条 rail 本来就空白）和「被 PATCH 成空工作区的损坏行」（该行已无法执行任何一轮对话，`service.rs:12137-12141`）。放宽门槛要动 `WorkspaceToolRail` 里硬编码的 `文件`/`变更` 两项（它们没有 `available` 开关），代价大于收益。

## 4. 数据来源

### 4.1 绑定目标解析：三分支纯函数

后端的真实规则是三分支，由 `knowledge_binding_target`（`service.rs:13251-13268`）与挂载分派（`service.rs:12310-12321`）共同决定：

| 条件 | 实际读写的绑定行 |
|---|---|
| `extra.companion_id` 存在 | `('companion', companion_id)` |
| 否则 `extra.preset_knowledge_binding === true` | `('conversation', conversation_id)` |
| 否则 | `('workpath', workpathKeyForConversation(extra))` |
| 终端会话 | `('workpath', workpathKeyForTerminal(session))` |

新增纯函数 `Workspace/KnowledgePanel/knowledgeBindingTarget.ts`：

```ts
export type SessionKnowledgeSource =
  | { kind: 'conversation'; conversationId: ConversationId; extra: Record<string, unknown> | undefined }
  | { kind: 'terminal'; session: Pick<ITerminalSession, 'cwd' | 'is_default_workpath'> };

export function resolveKnowledgeBindingTarget(
  source: SessionKnowledgeSource
): { kind: KnowledgeBindingKind; target_id: string };
```

复用既有纯函数 `workpathKeyForConversation` / `workpathKeyForTerminal`（`SessionList/utils/sessionWorkpath.ts:25-36`）—— 必须复用，否则面板读到的行会和 `KnowledgeControl` 写的行不是同一行。`preset_knowledge_binding` 是服务端独占字段（入站被 `strip_server_owned_preset_fields` 剥离，`routes.rs:131-145`；由 `service.rs:4186` 与 `companion.rs:1028` 置位），所以响应上可读。

### 4.2 终端必须用页面自己的 session 对象

`KnowledgeControl` 是拿 `target.id` 去 `useTerminalSessions()` 里查会话（`KnowledgeControl.tsx:183-187`），而那个 hook **故意过滤掉了归属于会话的终端**（`pages/terminal/useTerminalSessions.ts:13-18, 39-49`），这类终端会解析失败。`TerminalRightRegion` 本身持有完整的 `ITerminalSession`，直接传 `session` 进解析函数，不查表。

### 4.3 已知分歧：`KnowledgeControl` 缺第二分支（本次不修）

`KnowledgeControl.tsx:175-189` 的 `resolved` memo 只实现了第 1、3 分支，缺 `preset_knowledge_binding` 那一支。因此**预设创建的会话**在弹窗里改挂载会写进 workpath 行，而运行时读的是 conversation 行 —— 用户的修改静默不生效。这是既有 bug，不在本次范围：

- 新面板（只读）用 §4.1 的完整解析，**显示运行时真实挂载的库**。
- `KnowledgeControl`（可写）不动。改它等于在一个 UI 优化里变更写入行为，需要独立的回归验证。
- 后果记录在案：预设创建的会话下，弹窗列表与面板树可能不一致，面板是对的。

### 4.4 联结与刷新

`useSessionKnowledgeMounts(source)` 返回 `{ mounted, bases, loading, error }`：

1. `resolveKnowledgeBindingTarget(source)` → `{ kind, target_id }`
2. `ipcBridge.knowledge.getBinding.invoke({ kind, target_id })` → `kb_ids`（未命中时返回 `{enabled:false, kb_ids:[]}`，不是 404）
3. `ipcBridge.knowledge.listBases.invoke()` → 用 `kb_ids` 过滤并**按 `kb_ids` 的顺序**排列，得到根节点（`IKnowledgeBase` 提供 `name` / `root_path` / `root_exists` / `kind`，`ipcBridge.ts:5556-5578`）
4. 订阅 `onBindingChanged`（payload 自带完整 binding，可直接套用无需重取）+ `onBaseCreated` / `onBaseUpdated` / `onBaseDeleted` 触发 `listBases` 重取

不用 `useKnowledgeBase(id)`（`pages/knowledge/useKnowledge.ts:57-100`）—— 它每个库要 3 个请求，N 个挂载就是 3N。

会话/终端负载里没有任何知识库字段（`ui/src/common/config/storage.ts:250-270`、`ITerminalSession`），所以这两个请求是必需的，没有批量解析接口。

## 5. 复用清单：走 `extraTabs` 扩展位

右栏已有数据驱动的扩展位，本次一行契约都不用改：

| 复用点 | 位置 | 说明 |
|---|---|---|
| rail 图标槽 | `ChatLayout/WorkspaceToolRail.tsx:121-129` | `extraTabs?.map` 已渲染 `{key,title,icon}`，含 active 态、左侧 mini Tooltip、`aria-pressed`、视觉隐藏 label |
| 面板 body 槽 | `Workspace/WorkspaceRailBody.tsx:277-279, 583-585` | `activeExtraTab` 渲染在 `FlexFullContainer containerClassName='overflow-y-auto'` 内 —— 滚动免费获得；未知 key 会归一到 `'files'`（`:279-284`） |
| tab key 类型 | `Workspace/types.ts:211` | `WorkspaceTab = 'files' \| 'changes' \| (string & {})`，加 `'session-knowledge'` 无需改类型 |
| 面板开关状态 | `hooks/useWorkspacePanelTabs.ts:29-72` | 按 `SessionTarget` 的 localStorage + `WORKSPACE_PANEL_TAB_EVENT`，接受任意字符串 key |
| 收起/展开 | `hooks/useWorkspaceCollapse.ts` | 任何 extra tab 自动继承，含"再点一次收起" |
| 树纯函数 | `KnowledgeDetailPage/treeModel.ts` | `mergeKnowledgeTreeChildren`、`preserveKnowledgeTreeChildren` 等，零依赖且有单测 |
| 文件预览 | `Preview/context/PreviewContext.tsx:53` | `openPreview(content, 'markdown', meta)`；`'markdown'` 是合法 `PreviewContentType`（`common/types/office/preview.ts:11-21`），带标签页与同文件去重 |
| 文案 | `knowledge.detail.docs.expandAll` / `.collapseAll` / `knowledge.control.label` / `.mounted` / `knowledge.mount.rootMissing` | 双语均已存在 |
| 图标 | `BookOne`（`@icon-park/react`） | 应用既定的知识库字形（`KnowledgeControl.tsx:30`） |
| 唯一先例 | `components/ConversationTerminalPanel.tsx:36-503` | 目前唯一的 extra tab body：自带 props、自取数据、自渲染顶部工具行、自处理 loading/error/empty |

三类会话都已各自挂了 `PreviewProvider`（`ChatLayout/index.tsx:554` 覆盖普通与伙伴，`TerminalSessionPage.tsx:432` 覆盖终端），预览零新增管线。

**不复用** `useWorkspaceTree`（`Workspace/hooks/useWorkspaceTree.ts`）：它硬绑 `IDirOrFile`、单根（`flattenSingleRoot`），且会发 `WORKSPACE_HAS_FILES_EVENT` 驱动面板自动展开 —— 借用它会把那个信号泄漏出去。

## 6. 改动清单

### 6.1 新增

```
ui/src/renderer/pages/conversation/Workspace/KnowledgePanel/
├── index.tsx                       面板本体
├── knowledgeBindingTarget.ts       §4.1 纯函数
├── knowledgeBindingTarget.test.ts  三分支真值表（镜像 service.rs:13862-13932 的 Rust 单测）
├── useSessionKnowledgeMounts.ts    §4.4
└── sessionKnowledgePanel.test.ts   面板契约（§8）
```

落点理由：面板只由 `WorkspaceRailBody` 渲染，与它同目录；无文档规定跨三类会话共享的面板该放哪，此处显式定为约定。

**不新增 CSS 文件。** 只用 Uno 工具类与主题 token（`var(--color-text-*)` / `var(--color-border-*)` / `var(--color-fill-*)` / `rgba(var(--primary-6),…)`），规避 `check:dead-css` 的 7 类禁用写法与主题双属性陷阱（浅深色同时跨 `html[data-theme]` 与 `body[arco-theme]`，`hooks/system/useTheme.ts:11-14`）。

### 6.2 修改

| 文件 | 改动 |
|---|---|
| `KnowledgeDetailPage/treeModel.ts` | 上提并导出 `collectKnowledgeDirKeys`（现为 `index.tsx:100-111` 的模块私有函数） |
| `KnowledgeDetailPage/index.tsx` | 删除本地定义，改为 import；注意 `knowledgeDetailActionBar.test.ts` 按源码文本断言，需同步核对 |
| `ChatConversation.tsx` | 目前有**两份**独立的 `workspaceExtraTabs`（`:209-218` 与 `:633-646`），抽成 `hooks/useWorkspaceExtraTabs.ts` 供两处共用 |
| `TerminalWorkspaceRail.tsx` | props 与 `WorkspaceSource` 增加 `extraTabs` 透传（今天完全没有这个通道） |
| `TerminalSessionPage.tsx` | 构造 extraTabs 并传给 `WorkspaceToolRail`（`:208-226`，今天不传）与 `TerminalWorkspaceRail`；把 `:196-200` 硬编码的两路标题三元改成与 `ChatLayout/index.tsx:136-141` 相同的通用查找 |
| i18n 两语 | 新增键见 §7.6 |

新增 `ui/src/renderer/pages/conversation/hooks/useWorkspaceExtraTabs.ts`：入参 `conversation`，返回 `WorkspaceExtraTab[]`，内含既有的 `conversation-terminals` 与本次的 `session-knowledge` 两项。

`ChatConversation.tsx` 的两份 memo 是本次最容易出错的地方：只加一份会让一半会话类型（附件转写 / 伙伴 / acp 家族，或反之普通 nomi）静默看不到图标，且没有现成测试能抓住。抽 hook 是把这个坑一次性填掉，而不是再复制一遍。

注意两处 memo 的现有门槛并不相同：`:633-646` 门槛是 `conversation?.extra?.workspace`，而 `:209-218` 无门槛（它靠 `ChatLayout` 的 `workspaceEnabled` 兜住）。抽出的 hook 统一取"有工作区才产出 tab"，与 rail 的实际可见性对齐。

## 7. 面板行为

### 7.1 结构

```
┌────────────────────┐
│ 知识库              │  ← 面板头，标题取自 extra tab 的 title
├────────────────────┤
│ 已挂载 2 个知识库 ⊟ │  ← 面板内工具行（本次新增）
│ ▾ python基础        │  ← 根 = 知识库
│    ▸ notes          │
│    ▸ snapshots      │
│    README.md        │
│ ▸ xxxxx 挂载的知识库 │
└────────────────────┘
```

「全部展开 / 折叠」按钮放在**面板内的工具行**，不放面板头。面板头 `WorkspacePanelHeader.tsx:63-69` 的右侧动作槽写死了 `activeTab === 'files'` 且只认 `WorkspaceBindButton` / `WorkspaceOpenButton`，没有通用的 per-panel 动作槽；加一个要给 `WorkspaceExtraTab` 增字段并接到 `ChatLayout/index.tsx:453-464`、`MobileWorkspaceOverlay.tsx:63-74`、`TerminalSessionPage.tsx:190-206` 三处，还要同步三个源码文本断言测试。面板内工具行是既有先例（`ConversationTerminalPanel.tsx:458-468`）且契约零改动。

### 7.2 树键必须按知识库作用域

现有所有知识库树的 key 都是裸 `rel_path`（`KnowledgeDetailPage` 单根，不会撞）。多根必须加前缀，否则两个库里同名的 `README.md` 会共享展开/选中态：

- 根节点：`${knowledge_base_id}::`
- 子节点：`${knowledge_base_id}::${rel_path}`

知识库 id 是 UUID，不含 `::`，分隔符无歧义。每个节点还要能反查所属库（取 `root_path` 拼绝对路径、调 `listTree`），所以节点数据上挂 `knowledgeBaseId`。

### 7.3 展开语义

- **懒加载**：展开某目录 → `listTree({ knowledge_base_id, path })`，用 `mergeKnowledgeTreeChildren` 合并。
- **全部展开** = 每个根各展开**一层**：对每个挂载库取 `listTree({ knowledge_base_id })`（无 `path`），`expandedKeys` 置为全部根键。请求数 = 挂载数（通常 1–3），秒开。更深层级仍可手动逐级懒加载。
- **全部折叠** = `expandedKeys = []`，只剩根。
- **首次打开面板** = 等同一次「全部展开」，避免开屏只有两行根目录。该展开态**不持久化**：随面板卸载丢弃，下次打开重新展开一层。`useWorkspacePanelTabs` 只记"哪个 tab 是激活的"，不记树内部状态，本次不扩展它。
- 按钮态由 `expandedKeys` 是否覆盖全部根键派生（`ExpandUp` / `ExpandDown` 互换），加载中给 `loading`。

**明确不做递归全展开。** 详情页的 `handleExpandAllTreeNodes`（`index.tsx:629-656`）是每个目录一次 HTTP、无缓存无取消；rail 里 N 个根会放大成几十次请求。设计稿写的是"点击展开：全部根目录"，本设计取字面语义。

### 7.4 点击文件

单击文件 → `readFile({ knowledge_base_id, path })` → `openPreview(content, 'markdown', meta)`。`meta` 沿用 `useWorkspaceFileOps.ts:300-350` 的形状，其中：

- `file_path` 用 `${base.root_path}/${relPath}` —— **必须是绝对路径**，正文里的本地图片才能解析
- `editable: false`
- `title` 用 `${base.name} / ${叶子名}`

树常驻可见，可连续点多个文件对比（预览列自带标签页与同文件去重）。

**树里天然只有 markdown。** 后端 `list_tree_level` 仅在 `is_md()` 为真时产出文件节点（`crates/backend/nomifun-knowledge/src/service.rs:5935`，`is_md` = 扩展名 `md`，`:5081-5085`），图片 / PDF / 代码不可能成为节点。所以预览类型恒为 `'markdown'`，不需要 MIME 分支。

### 7.5 空态与错误：三种状态必须分开说

| 状态 | 依据 | 文案取向 |
|---|---|---|
| 源目录不存在 | `base.root_exists === false` | 复用 `knowledge.mount.rootMissing`「目录不可用」+ `rootMissingHint` |
| 某库无可预览文档 | `listTree` 返回空数组 | **中性**："没有可预览的文档"。不可写成"这个知识库是空的" |
| 读文件失败 | `readFile` 抛错 | 一次 `Message.error`，不影响树 |

第二条的中性措辞是必需的：`list_files` / `list_tree` 在 6 秒遍历预算耗尽时**静默返回空数组**（`nomifun-knowledge/src/service.rs:1272-1302`），"真的空"与"慢盘/网络盘超时"在响应上不可区分。断言"是空的"会说谎。

`readFile` 无字节上限，`read_to_string` 20 秒超时，非 UTF-8 直接 500（`service.rs:1354-1390`）。50MB 的 `.md` 会整份进 JSON。本次不加前端体积闸门（与详情页现状一致），但错误必须被吞掉而不是打断树。

### 7.6 文案与 i18n

绝大部分文案复用既有键（双语均已存在，已核对 `locales/{en-US,zh-CN}/knowledge.json`）：

| 用途 | 键 | zh-CN / en-US |
|---|---|---|
| rail tooltip 与面板头标题 | `knowledge.control.label` | 知识库 / Knowledge |
| 工具行挂载计数 | `knowledge.control.mounted` | 已挂载 {{count}} 个知识库 / {{count}} base(s) mounted |
| 展开按钮 | `knowledge.detail.docs.expandAll` | 全部展开 / Expand all |
| 折叠按钮 | `knowledge.detail.docs.collapseAll` | 全部折叠 / Collapse all |
| 源目录不存在 | `knowledge.mount.rootMissing` + `.rootMissingHint` | 目录不可用 / … |

**唯一新增键**（两语同时加，随后跑 `bun run gen:i18n` 重生成 `i18n-keys.d.ts`）：

| 键 | zh-CN | en-US |
|---|---|---|
| `knowledge.session.noDocs` | 没有可预览的文档 | No previewable documents |

读文件失败沿用全仓惯例 `Message.error(String(e))`，不新增键。`locales/<lang>/index.ts` 才是命名空间注册表（`ui/src/common/config/i18n-config.json` 的 `modules` 数组已过期，不是注册表）；`knowledge` 命名空间已注册，无需改动。

### 7.7 权限现状（照实记录，本次不改）

`/api/knowledge/*` 整个路由包在 `protect_instance_owner` 里（`crates/backend/nomifun-app/src/router/routes.rs:851-856`），**没有"该会话是否挂载了这个库"的服务端校验** —— 挂载限制只在面向智能体的 MCP 路径上（`nomifun-knowledge/src/mcp_server.rs:200-315`）。`kb_ids` 过滤是 UI 侧的唯一闸门。这与 `/knowledge` 详情页现状一致，不构成本次的新增暴露面。

## 8. 测试

前端测试**不在** `bun run check` 链里（`package.json:32` 只到 `help --check`），必须另跑 `bun test --cwd ui`。本仓测试是 `bun:test` + 就地放置 + 以 `readFileSync` 源码断言为主；无 testing-library，且**没有任何现存测试渲染过 Arco `Tree`**。

| 测试 | 断言 |
|---|---|
| `knowledgeBindingTarget.test.ts` | 三分支真值表：`companion_id` 优先于 `preset_knowledge_binding` 优先于 workpath；终端走 `workpathKeyForTerminal`；镜像 `service.rs:13862-13932` 的 Rust 用例 |
| `knowledgeTreeModel.test.ts`（补充） | 上提后的 `collectKnowledgeDirKeys`：只收目录、递归、跳过文件 |
| `sessionKnowledgePanel.test.ts`（新增，源码契约） | ① 可见判定含 `enabled` 与 `kb_ids.length` 双条件；② 树键带 `${kb_id}::` 前缀；③「全部展开」只置根键、不递归（源码内不得出现递归 `listTree`）；④ 只读 —— 源码不得 import `writeFile` / `deleteFile` / `createFolder` / `renameTreeEntry`；⑤ 空态文案不含"空的"式断言 |
| `ChatConversation` 契约（新增或并入既有） | 两处 extraTabs 来自同一个 `useWorkspaceExtraTabs`，防止再次分叉 |
| `TerminalSessionPage.structure.test.ts`（更新） | `:22-27` 断言 `className='!bg-1 relative layout-sider'` 紧邻 `<WorkspaceToolRail`，加 `extraTabs` 后需同步 |
| `knowledgeDetailActionBar.test.ts`（核对） | 按源码文本断言，`collectKnowledgeDirKeys` 上提后需确认未破坏 |

## 9. 门禁与陷阱

- `bun run check`（9 项）+ `bun test --cwd ui`（**必须显式跑**）。
- **图标导入必须单行**：`import { BookOne } from '@icon-park/react';`。Vite 插件（`ui/vite.config.ts:28-51`）只重写 `.tsx` 里单行、空格填充的具名导入；多行导入会**静默丢掉** HOC 默认值（尺寸、`iconColors.secondary` 填充、`cursor-pointer`），而 `check:icons` 抓不到这个（它只禁别名与命名空间导入）。渲染用 `<BookOne size={18} />` 与 rail 同侪一致。
- `check:dead-css` 无豁免名单：禁 `text-[rgb(var(--danger-6))]` 这类 ramp 写法、`bg-bg-1` / `text-text-*` 双前缀、`border-b-base`、`border-b-2`，带宽度与颜色的 border 类必须配 `border-solid`。
- `check:theme` 只读 `pages/settings/DisplaySettings/presets/*.css`，看不到本面板 —— 但预设是最后注入且每条声明带 `!important`，所以颜色一律走 token，绝不写字面值。
- `check:agent-vocabulary` 禁 `orchestrat*` / `sub-agent` / `fleet*` / `agent-cluster` 出现在标识符、类名与文档标题里。
- 新文件带 SPDX `@license` 头（仓内 434/510 个 `.tsx` 有，属强约定）。
- 线型命名：参数必须是 `{ knowledge_base_id: KnowledgeBaseId }`，禁用 `/api/knowledge/bases/${p.id}`（`ipcBridge.resource-wire.test.ts:20-42`）。
- 终端在移动端**没有** rail（`TerminalSessionPage.tsx:172, 208` 都是 `!isMobile`，也没有终端版 `MobileWorkspaceOverlay`）。所以终端上的知识库面板是桌面独占。这是既有状况，本次记录不修。

## 10. 已知代价

1. **预设会话下弹窗与面板可能不一致**（§4.3）。面板是对的，弹窗是既有 bug。
2. **终端移动端无入口**（§9 末条），既有状况。
3. **「全部展开」只到一层**（§7.3）。更深层级需手动展开，换来的是可预期的请求数。
4. **空态无法区分"真空"与"遍历超时"**（§7.5），受后端行为限制，用中性文案承担。
5. **大文件无前端体积闸门**（§7.5），与详情页现状一致。
