# 需求列表排序 + 体验打磨 设计

日期：2026-06-28
状态：已批准，实施中

## 背景与问题

用户反馈需求平台列表页（需求工作区 `WorkspacePage`，卡片行式列表）使用体验差，期望具备：ID 排序、指定字段排序、批量删除、指定字段过滤、分页；并指出分页图标配色饱和度过高、不美观。

探索结论：该列表**已具备** 标签/状态/关键词搜索筛选、服务端分页（Arco `Pagination`）、批量删除（勾选行 + 批量操作条）。**真正缺失的是排序**——仓储层 `list()` 的 `ORDER BY` 写死为 `sort_seq ASC, priority DESC, created_at ASC`，且 `ListRequirementsParams` / `ListRequirementsQuery` 无任何排序参数。分页配色直接采用 Arco 默认主色（高饱和），无自定义覆盖；`Pagination` 全应用仅 `RequirementListView.tsx` 一处使用。

## 范围（用户确认）

- 核心：新增**排序**——卡片行外观不变，在筛选行右侧加「排序字段下拉 + 升/降序切换」。字段：ID、创建时间、更新时间、状态。
- 打磨：分页激活态/箭头配色调柔和；批量删除新增「全选本页 / 清除」；分页大数据量开启快速跳页。
- 维持现状：筛选字段不扩展（标签/状态/搜索）；不改回表格；分页配色改动作用域限于需求列表，不全局。

## 架构与数据流

排序为**服务端排序**（只排当前页无意义）。新增查询参数 `order_by` + `order`，沿现有链路打通：

```
UI 排序下拉/升降序
  → WorkspacePage orderBy/order state
  → useRequirements({ ..., order_by, order })
  → ipcBridge.requirements.list（拼进 URL query）
  → axum list_requirements → ListRequirementsQuery{ order_by, order }
  → requirement_service.list → ListRequirementsParams{ order_by, order }
  → SqliteRequirementRepository::list → 白名单 ORDER BY <col> <dir>[, id <dir>]
```

UI 列表请求走 HTTP（`httpGet` → `fetch(baseUrl + path)`，桌面/网页均透传 query 到 axum）。gateway 的 `caps_requirement.rs` 仅服务 AI 工具 `nomi_requirement_list`，不在 UI 列表路径上。

### 安全

`order_by` 在仓储层用**白名单**映射为真实列名（`id|created_at|updated_at|status`），非法值/缺省回退默认队列序；`order` 仅接受 `asc|desc`，缺省 `desc`。绝不把用户输入拼进 SQL。

### 稳定分页

按非唯一列（如 `status`）排序时追加 `id <同向>` 作为最终 tiebreaker，保证翻页确定、无重复/漏项。`id` 唯一，无需 tiebreaker。

### 默认行为不变

排序下拉默认值「默认顺序」时不发送 `order_by/order`，后端保持 `sort_seq ASC, priority DESC, created_at ASC`（AutoWork 队列序）。看板视图始终用默认序，不受影响。

## 改动清单

### 后端（Rust）

| 文件 | 改动 |
|---|---|
| `nomifun-api-types/src/requirement.rs` | `ListRequirementsQuery` 增 `order_by: Option<String>`、`order: Option<String>`（`#[serde(default)]`） |
| `nomifun-db/src/repository/requirement.rs` | `ListRequirementsParams` 增 `order_by`、`order` 字段（结构体已 `#[derive(Default)]`） |
| `nomifun-db/src/repository/sqlite_requirement.rs` | 新增 `build_order_clause(order_by, order)` 白名单构造；`list()` 用它替换写死的 ORDER BY；新增单测 |
| `nomifun-requirement/src/service.rs` | `list()` 把 `query.order_by/order` 映射进 `ListRequirementsParams` |
| `nomifun-gateway/src/caps_requirement.rs` | 两处 `ListRequirementsQuery { .. }` 字面量补 `order_by: None, order: None`（仅编译兼容） |

`build_order_clause` 逻辑：
- 方向：`asc`→`ASC`，`desc`→`DESC`，其余→`DESC`。
- 列：`id|created_at|updated_at|status` 命中 → 对应列；否则（含 None）→ 返回默认队列序整句。
- `id` → `ORDER BY id <dir>`；其它命中列 → `ORDER BY <col> <dir>, id <dir>`。

### 前端（TS / React）

| 文件 | 改动 |
|---|---|
| `ui/src/common/adapter/ipcBridge.ts` | `IListRequirementsParams` 增 `order_by?: RequirementOrderBy`、`order?: 'asc'\|'desc'`；query 构造补两行；导出 `RequirementOrderBy` 类型 |
| `WorkspacePage/index.tsx` | 新增 `orderBy`/`order` 本地 state（不入 URL，变更重置 page=1）；`selectAllOnPage`/`clearSelection` 处理；传参给 `useRequirements`、`RequirementFilters`、`RequirementListView` |
| `WorkspacePage/RequirementFilters.tsx` | 右侧加排序字段 `Select`（默认顺序/ID/创建/更新/状态）+ 升降序切换 `Button`（选默认顺序时禁用） |
| `WorkspacePage/RequirementListView.tsx` | 行上方加纤细 header：全选本页 `Checkbox`（indeterminate）+「共 N 条 / 已选 M / 清除」；`Pagination` 加 `className='requirements-pagination'`、`showJumper` |
| `ui/src/renderer/styles/arco-override.css` | 追加 `.requirements-pagination` 作用域样式：激活页码改柔和填充 + text-1 文字，箭头用 text-2，hover 轻微提亮（明暗主题均用 token） |

### i18n

`requirements.json`（zh-CN + en-US）新增：
- `sort`: `label / default / byId / byCreatedAt / byUpdatedAt / byStatus / asc / desc`
- `selection`: `selectAllPage / clear / totalCount({{count}}) / selectedCount({{count}})`

随后 `bun run gen:i18n` 重新生成 `i18n-keys.d.ts`，`bun run check:i18n` 校验。

## 测试与验证

- 仓储层单测（in-memory sqlite，沿用现有模式）：各排序字段升/降序结果顺序、`status` 排序的 `id` tiebreaker 稳定性、非法 `order_by` 回退默认、与过滤/分页组合。
- 前端：`bun run typecheck`；`bun run check`（i18n/theme 契约）。
- 人工：实跑确认排序生效、分页配色变柔和、全选本页/清除可用。

## 范围之外

扩展筛选字段、列表改回表格、分页配色全局化——本次均不做。
