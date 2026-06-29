# 桌面伙伴记忆：共享/私有作用域 + 编辑 + 多入口 设计

> 状态：已确认（方案 A 整体交付）。日期：2026-06-29。

## 背景与问题

桌面伙伴的记忆有两套互不相干的系统：

| 系统 | 存储 | 类别 | 用途 |
|---|---|---|---|
| **伙伴记忆**（本设计目标） | SQLite `companion_memories`（`crates/backend/nomifun-companion`） | profile/preference/knowledge/episode/task/affective 六类 | 注入伙伴人格 prompt + 对话中 `recall_memories` |
| Agent 编码记忆 | 文件 `MEMORY.md`（`crates/agent/nomi-memory`） | user/feedback/project/reference | Claude-Code 式长期记忆，不在本设计范围 |

排查发现"找不到记忆编辑入口"是三个问题叠加：

1. **入口埋得深**：记忆 UI（`MemoriesTab.tsx`）藏在 nomi 页"共享(Shared)"域下，页面默认落"伙伴/概览"域，需先切换无"记忆"字样的域单选才看得到（`nomi/index.tsx:53,167-171`）。
2. **悬浮伙伴到不了记忆**：悬浮窗右键菜单只有 4 项（打开对话/打开设置/清除未读/隐藏），"打开设置"跳 `tab=settings` 而非 memories（`companionNativeMenu.ts:16-23`、`companion/index.tsx:1304-1321`）。
3. **没有编辑按钮**：`MemoriesTab` 只能 增/置顶/归档/删除，内容是只读 `<div>`（`MemoriesTab.tsx:146`）。但 `updateMemory` IPC 已支持 `content?`，后端 PUT 已用 COALESCE 落库（`store.rs` update_memory）——所以"编辑内容"本是纯前端缺口。

附带发现的代码设计问题（本设计一并修复）：
- **脱敏不对称**：`insert_memory` 跑 `nomi_redact::redact_secrets`，`update_memory` 不跑。
- **更新无内容校验**：`add_memory` trim/拒空，`update_memory` 不会。
- **事件不对称**：只有 `companion.memory-created` 且只在对话保存路径发；HTTP 新增/编辑/删除都不发事件，前端只监听 created。
- **作用域列是死列**：`companion_memories` 有 `scope_kind TEXT DEFAULT 'user'` + `scope_companion_id TEXT`（v2→v3 迁移加），但 struct 不含、`row_to_memory` 不读、所有写/读/注入/recall 都不用 → 记忆事实上全局共享。
- **`companion_skills` 已把同一套作用域机制完整接通**（`scope_kind` `'user'`/`'companion'` + `scope_companion_id` `''`=共享，查询 `WHERE scope_companion_id=? OR scope_kind='user'`，`SkillScope{Shared,Companion(id)}` 枚举）——本设计照搬该蓝图到记忆。

## 目标

1. 伙伴记忆区分**共享**（所有伙伴可见）与**私有**（仅归属伙伴可见），两者皆可编辑。
2. 编辑可改 **内容 + 共享/私有归属**（不改 kind/tags/importance/strength）。
3. 三个入口：**悬浮伙伴右键菜单**、**侧边栏/nomi 页一级可见**、**对话窗口内**。
4. 顺带修复上述后端安全/事件隐患。

非目标：语义检索/embedding；手动编辑 strength/importance/kind/tags；回溯改写进行中对话已烘焙的 prompt。

## 概念模型

| 作用域 | `scope_kind` | `scope_companion_id` | 可见范围 |
|---|---|---|---|
| **共享** | `'user'` | `''` | 所有伙伴 |
| **私有** | `'companion'` | 归属伙伴 id | 仅该伙伴 |

伙伴 C 可见记忆 = `scope_kind='user' OR scope_companion_id = C`（与 `companion_skills` 一致）。

**默认归属**：
- 对话保存（`CompanionStoreSink::save`，source=`chat`）→ **私有给该伙伴**（复用 owning-companion 解析）。
- 学习中枢（`learner`，source=`learn`）→ **共享**。
- 手动新增（route/UI）→ 请求携带，UI 选择，默认共享。

## 实现分层

### A. 数据层 `crates/backend/nomifun-companion/src/store.rs`
- 新增 `MemoryScope { Shared, Companion(String) }` 枚举 + `scope_columns()`/`from_columns()` 辅助（镜像 `SkillScope` / `service.rs:scope_for`）。
- `CompanionMemory` 加字段 `scope_kind: String`、`scope_companion_id: String`。
- `row_to_memory`：读出两列，`scope_companion_id` 用 `COALESCE(...,'')` 归一 NULL→`''`。
- 迁移：新增幂等回填步骤 `UPDATE companion_memories SET scope_companion_id='' WHERE scope_companion_id IS NULL`（旧行 `scope_kind='user'` 即共享，符合现状）。
- `insert_memory`：签名加 `scope: MemoryScope`，INSERT 列含 scope_kind/scope_companion_id。
- `insert_memory_raw`（import）：INSERT 列含两列（struct 已带，import 可往返）。
- `memories_for_injection`：签名加 `companion_id: &str`，两个 SELECT 加 `AND (scope_kind='user' OR scope_companion_id = ?)`。
- `MemoryFilter`：加 `scope_companion_id: Option<String>`；`list_memories` 据此加同款谓词。
- `update_memory`：除 content/pinned/status，接受可选 `scope: Option<MemoryScope>`；**对 content 重跑 `nomi_redact::redact_secrets`**；**trim 拒空**。

### B. 服务/路由/事件 `service.rs` / `routes.rs` / `events.rs` / `companion.rs` / `learner.rs` / `export.rs`
- `service.add_memory`：加 `scope` 参数并下传；保留 kind/content 校验。
- `service.update_memory`：下传 scope；status 枚举校验（已有）+ content trim；若 `scope_kind='companion'`，校验 companion_id 存在。
- `routes.rs`：
  - `AddMemoryRequest` 加 `scope_companion_id: Option<String>`（`''`/缺省=共享）。
  - `UpdateMemoryRequest` 加 `scope_kind: Option<String>` / `scope_companion_id: Option<String>`。
- `events.rs`：新增 `emit_memory_updated(memory)`→`companion.memory-updated`、`emit_memory_deleted(id)`→`companion.memory-deleted`。HTTP add 路径补发 `memory-created`；PUT 发 updated；DELETE 发 deleted。
- 默认归属接线：`CompanionStoreSink::save`→`Companion(owning_id)`；`learner` insert→`Shared`；route add→请求。
- `build_companion_system_prompt`：加 `companion_id` 形参并下传 `memories_for_injection`；更新全部调用点。
- recall：把伙伴 id 传进 `MemoryFilter.scope_companion_id`，`recall` 返回 共享+该伙伴私有。

### C. IPC/TS `ui/src/common/adapter/ipcBridge.ts`
- `ICompanionMemory` 加 `scope_kind: 'user'|'companion'`、`scope_companion_id: string`。
- `addMemory`/`updateMemory`/`listMemories` payload 加作用域字段。
- 新增 `onMemoryUpdated`、`onMemoryDeleted` WS 监听。

### D. 前端
- **D1 编辑（`MemoriesTab.tsx`）**：每行加"编辑"按钮 → 弹窗改 content + 归属选择器（共享/私有给某伙伴）；工具栏加作用域筛选（全部/共享/仅当前伙伴）；每行作用域徽标；新增弹窗也带归属选择器；订阅 updated/deleted 实时刷新；加"编辑只影响新对话+实时 recall"的说明文案。
- **D2 入口① 悬浮伙伴右键菜单**（`companionNativeMenu.ts` / `companion/index.tsx`）：`CompanionMenuAction` 加 `'open-memories'`；菜单加"打开记忆"项 → `openMainAt('/nomi?companion={id}&tab=memories')`；更新 `companionNativeMenu.test.ts`。
- **D3 入口② 侧边栏/nomi 可发现性**（`nomi/index.tsx`）：把 `memories` 从"共享"域移到"**伙伴**"域，成为一级"记忆"标签；scope-aware：选中伙伴时默认显示 共享+该伙伴私有，作用域筛选保留"全部伙伴"。
- **D4 入口③ 对话窗口内**（`companion/index.tsx`）：对话中 `onMemoryCreated`/updated 触发显示低调小条"记下了：<摘要>"+"编辑/管理"动作 → `openMainAt('/nomi?companion={id}&tab=memories')`；聊天栏加"记忆"小入口同样跳主窗口。不在 240×214 小窗内行内编辑。
- **i18n**：新 key（`nomi.memories.edit`/`saved`/`scope`/`scopeShared`/`scopePrivate`/`scopeFilterAll|Shared|Private`/`editHint`、`nomi.menuOpenMemories` 等）补 en-US + zh-CN `nomi.json`，重新生成 `i18n-keys.d.ts`。

## 数据流

- 新增(手动)：UI 弹窗(content+scope) → `POST /api/companion/memories` → `service.add_memory(scope)` → `insert_memory(..,scope)` → emit created → 各面板刷新。
- 新增(对话)：伙伴 `save_memory` 工具 → `CompanionStoreSink::save` 解析归属伙伴 → `insert_memory(scope=Companion(id))` → emit created。
- 新增(学习)：`learner` → `insert_memory(scope=Shared)`。
- 编辑：UI 弹窗 → `PUT /:id {content?,scope_kind?,scope_companion_id?}` → `service.update_memory`(redact+trim) → `store.update_memory`(COALESCE) → emit updated → 刷新。
- 注入：`build_companion_system_prompt(companion_id)` → `memories_for_injection(companion_id,..)` → 共享+该伙伴私有，烘焙进新对话。
- recall：`recall_memories` → `list_memories` 带 scope 过滤。

## 错误处理 & 安全
- update：trim 拒空（400）；重跑脱敏；status 枚举校验；scope_kind='companion' 时校验 companion_id 存在（否则 400）。
- 迁移回填幂等。
- 持久化快照：编辑只影响新对话 + 实时 recall，不回溯改写在飞对话——UI 文案说明。
- DELETE 未知 id 返回 404（与 PUT 一致）——可选清理项。
- `insert_memory_raw`（import）保持不脱敏（高保真导入），文档注明。

## 测试

**Rust**：
- store：带 scope 插入写两列；`row_to_memory` 读出；NULL→`''` 归一；`memories_for_injection` 过滤（伙伴见共享+自己私有，不见他人私有）；`list_memories` scope 过滤；`update_memory` 脱敏 + 拒空 + 改 scope。
- service：三种默认归属（对话→私有、学习→共享、手动→请求）。
- events：created/updated/deleted 在对应路径发射。
- 迁移幂等。

**TS / 集成**：
- `companionNativeMenu.test.ts`：菜单含新"打开记忆"项与顺序。
- `MemoriesTab`：编辑流调用 `updateMemory` 带 content+scope；作用域筛选；徽标渲染（若有组件测试）。
- `bun run check`：typecheck + i18n 双语 key + theme contract。

## 涉及文件
后端：`store.rs / service.rs / routes.rs / events.rs / companion.rs / learner.rs / export.rs`（+ 迁移）。
IPC：`ipcBridge.ts`。
前端：`MemoriesTab.tsx / nomi/index.tsx / companion/index.tsx / companionNativeMenu.ts(+test) /`（必要时 Sider）。
i18n：`locales/en-US/nomi.json`、`locales/zh-CN/nomi.json` + 重新生成 `i18n-keys.d.ts`。

## 实现备注（落地时的取舍，与上文设计一致，细节微调）

- **无需新增迁移**：旧行 `scope_companion_id` 为 NULL，`row_to_memory` 用 `try_get::<Option<String>>` 归一成 `''`，配合 `scope_kind='user' OR scope_companion_id=?` 查询即正确；省去回填迁移、不动 `STORE_VERSION`。
- **`insert_memory` 保留为共享包装**：新增 `insert_memory_scoped(..., scope)`，`insert_memory(...)` = 共享包装，最小化对学习器/既有测试的冲击。
- **私有写入跳过去重**：`add_memory`/对话保存在私有作用域下跳过 `find_similar_active` 合并，避免把私有记忆误并进共享或他人记忆。
- **recall 作用域**：`CompanionMemorySink::recall` 增 `conversation_id`，由会话归属伙伴解析作用域（与注入一致）。
- **对话内入口（5d）**：实现为悬浮窗「空闲时」低调气泡提示「📝 已记住：…」（避免覆盖进行中的回复气泡），编辑/管理经右键「打开记忆」直达 scope-aware 记忆页。
- **预存且无关的破损**：集成测试 `nomifun-ai-agent/tests/factory_provider_integration.rs` 因更早提交给 `BuildTaskOptions` 增了 `conversation_created_at` 字段而未同步，本就编译失败（已 stash 验证与本改动无关），不在本次范围内修复。

