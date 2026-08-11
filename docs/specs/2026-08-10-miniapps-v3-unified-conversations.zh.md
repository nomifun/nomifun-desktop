# 小程序 v3 — 会话回归统一 · 返工决策

日期:2026-08-10
状态:决策定稿(用户逐项拍板),随码实施
前置:[`2026-08-09-miniapps.zh.md`](2026-08-09-miniapps.zh.md)(v1)、
[`2026-08-10-miniapps-v2-workspace.zh.md`](2026-08-10-miniapps-v2-workspace.zh.md)(v2)。
本文**取代** v2 的 D10、D11、D13 与 §7 的详情页形态,并作废 v2 §9.5、§9.6。
v2 的 D8(存储分层)、D12(显式发布)、D14(导入)与 §9.1–§9.4 的存储规则**继续有效**;
v1 的 D1(单文件自包含)、D3(免认证 serve + opaque origin)、D4(Nomi 引擎)继续有效。

## 1. 为什么返工

v2 把小程序做成了一个**自带会话的子系统**:每个小程序绑一个长期迭代会话、会话的工作区被
重定向进小程序目录、创建流程先建草稿行再落地详情页、详情页右侧内嵌一整个聊天面板、这些
会话还要从会话列表里藏起来。代价是:

1. **会话被分裂成两套。** 用户的会话散落在两个地方(主会话页与小程序详情页),历史、
   搜索、列表、删除各有一套语义,而详情页内嵌的聊天面板 import 的正是同一套
   `pages/conversation/**` 模块图 —— v2 §6 记录的那次路由级加载失败就出在这条线上。
2. **系统复杂度与收益不成比例。** 绑定会话带来了幂等置备、悬空自愈、条件写入、级联删除、
   列表过滤、双 marker(`extra.miniapp` / `extra.miniapp_iteration`)六类机制,而它们服务的
   只是"下次改这个小程序时接着上次聊"。
3. **"即时迭代"是个伪需求。** 已发布版本本来就与工作副本分离,用户真正要的是"改完让它生效",
   即一个发布动作,而不是一个常驻会话。

## 2. 决策(用户拍板)

| # | 决策点 | 结论 |
|---|--------|------|
| D15 | 绑定迭代会话(**取代 D10**) | **彻底删除**。`miniapps` 不再记录任何用于迭代的会话引用;置备/查询迭代会话的两条路由、幂等三分支、悬空自愈、条件写入一并删除 |
| D16 | 会话工作空间(**取代 D11**) | 小程序相关会话就是**普通会话、普通工作区**(托管临时工作区)。会话的路径重新散落在用户各自的会话里,不再被重定向进小程序目录 |
| D17 | 创建流程(**取代 D13**) | 「创建小程序」= 新建**普通 Nomi 会话** + 客户端注入构建提示词 → 落地 `/conversation/:id`。**不预建草稿行**;`miniapps` 行在首次「发布」时才产生 |
| D18 | 详情页形态(**取代 v2 §7**) | `/mini-apps/:id` 回归**单栏**:`MiniAppFrame` + 工具栏。删除 `ContentAside` 分栏与面板内聊天 |
| D19 | 迭代入口 | 库页/详情页「继续迭代」:先置备工作副本并取回**绝对源码路径**,再新建一个**普通会话**,首条消息自动写好「小程序 id + 源码路径 + 先读一遍再改」,落地 `/conversation/:id` |
| D20 | 发布动作(承接 D12) | 两条:①详情页「发布」——工作副本 → 快照;②会话预览面板「发布为新的小程序 / 替换已有小程序」——目标由用户显式选择,替换时**同步刷新工作副本**(v2 §9.1 的规则适用于这条路径) |
| D21 | 会话列表 | **不再过滤**。小程序会话就是普通会话,正常出现在列表里、正常删除 |
| D22 | 产物路径(用户确认保留) | `{work_dir}/miniapps/{miniapp_id}/miniapp.html` 保留为产物的专属托管路径,惰性物化。这正是 D19 里交给模型去定位的源码 |

### D22 与 D16 并存不矛盾

两者说的是不同的东西,这一点是整个返工的支点:

- **产物有专属路径**:小程序的源码归小程序自己管,与创建它的那次会话无关,所以会话被删掉
  也不影响产物。
- **会话没有专属路径**:会话不再被重定向去那个目录写文件。迭代会话是普通会话,它只是
  **被告知**了那个绝对路径,然后用普通文件工具去读改它 —— 桌面 surface 上每个会话的
  `file_authority` 本来就是 `PathAuthority::Unrestricted`(v2 §10 已记录),所以这不需要
  任何新能力。

## 3. 数据模型:回退一列

v2 的迁移 `029_miniapp_iteration.sql` **未提交**,因此就地改写而不是追加 030 ——
改写后只剩一列,文件随之改名为 `029_miniapp_published_at.sql`(描述与内容一致;
`sqlx::migrate!` 嵌入整个目录,血缘校验比对的是 version + checksum,没有代码按名引用它):

```sql
-- 只保留发布时间戳
ALTER TABLE miniapps ADD COLUMN published_at INTEGER;
```

删除:`iteration_conversation_id` 列、它的 UUIDv7 CHECK、
`idx_miniapps_iteration_conversation_id` 索引、`id_schema_contract.rs` 的
`LOGICAL_REFERENCES` 条目、`sqlite_conversation.rs` SET_NULL 块里那条
`UPDATE miniapps SET iteration_conversation_id = NULL`。

`source_conversation_id`(v1 迁移 028,**已提交**)保留,但语义降级为**纯溯源**:
不再用于任何跳转,也不再决定固化行为;它唯一的读者是「替换已有小程序」目标选择器里的
**默认选中项**(本会话此前发布过哪个,就默认选它)。删除该列需要新迁移,收益不足,不做。

## 4. HTTP 契约:净减两条

| 变更 | 路由 |
|---|---|
| **删除** | `POST /api/miniapps/{id}/iteration-session` |
| **删除** | `GET /api/miniapps/{id}/iteration-session` |
| **新增** | `POST /api/miniapps/{id}/workspace` — 幂等置备工作目录与工作副本,返回**绝对源码路径**(供 D19 写进首条消息)。不创建任何会话 |
| 保留 | `GET/POST /api/miniapps`、`GET/PUT/DELETE /api/miniapps/{id}`、`POST /api/miniapps/{id}/publish`、`POST /api/miniapps/import`、`POST /api/miniapps/validate`、`GET /api/miniapps/{id}/serve` |

`MiniAppResponse`:删除 `iteration_conversation_id`,保留 `published_at` 与
`has_unpublished_changes`。响应仍然不含 HTML 正文。

契约版本:**保持 18**。已发布基线是 `HEAD` 的 17,v2 那一层从未提交,所以本次仍然只是
17 → 18 这一次跨越;`ipcBridge.miniapp-wire.test.ts` 里 per-app 路由计数的断言按新集合更新。

客户端仍然**永不传路径**:`/workspace` 由后端从 `miniapp_id` 推导,`resolve_within_miniapps`
逃逸守卫照旧在每次打开前调用。客户端只是**读回**这个路径用于组织首条消息。

## 5. 提示词:收敛到一处

v2 §10 记下的隐患("构建提示词在 TS 与 Rust 各存一份,没有校验闩")本次顺手消除:

- **构建提示词**留在 `ui/src/renderer/pages/miniApps/contract.ts`(创建流程回归客户端注入
  `extra.system_prompt`,服务端不再生成会话)。
- **迭代首条消息**同样在 `contract.ts` 组装(它是一条用户可见消息,要走 i18n),内容:小程序
  名称与 id、绝对源码路径、"先完整读一遍再改"、"只改这一个文件"、"改完由用户点发布生效"。
- `crates/backend/nomifun-miniapp/src/prompt.rs` 在两条会话都回归客户端后应当**无引用** ——
  确认后整文件删除。

## 6. 清理清单(必须零残留)

删除:`iteration.rs`、`prompt.rs`(确认无引用后)、`MiniAppIterationPanel.tsx` 及其结构测试、
迭代会话相关的 e2e 测试、`extra.miniapp_iteration` marker 全链路、`conversationListFilter`
的小程序过滤及其测试、`ContentAside` 为分栏所做的改动(若无他处依赖则整体回退)、草稿占位
文档常量与 `draft.*` 文案、`iterate.panelTitle/panelSubtitle/preparing/openFailed/
sessionMissing/chatFailed/chatFailedHint`、`composer.workspaceNotice*`。

保留:库页面、单栏运行页、免认证 serve、导入与 Rust 侧校验、只读右侧快捷面板、
`MiniAppFrame`(含加载看护)、发布与未发布状态、`miniapp_workspace.rs`、`fsio.rs`。

`miniApps.iterate.toggle`(「继续迭代」)保留 —— D19 的入口就是它。

## 7. 验收

- Rust:`cargo test -p nomifun-db --test miniapps_schema`、`--test id_schema_contract`、
  `cargo test -p nomifun-miniapp`、`cargo test -p nomifun-app --test miniapp_e2e`。
- 前端:`bun run typecheck`、`bun test`、`bun run check` 全绿、`bun run build:ui`。
- 端到端:起 `nomifun-web` 走通 创建→发布→列表→serve→置备工作区→替换→删除,并确认
  删除小程序会带走 `{work_dir}/miniapps/{id}/`。
- 人工:桌面端点开左侧「小程序」任一卡片能看到应用在跑;点「继续迭代」落地的是
  `/conversation/:id` 且该会话出现在会话列表里。

### 本次返工:实际跑过的验证(2026-08-10)

自动化闸门全绿,逐条:

| 闸门 | 结果 |
|---|---|
| `cargo test -p nomifun-db` | 21 个测试二进制全绿(含 `miniapps_schema` 4、`id_schema_contract` 22) |
| `cargo test -p nomifun-miniapp` | 56 passed(含 `provisioning_the_workspace_answers_the_absolute_source_path`、`deleting_an_app_removes_its_working_directory`、`the_escape_guard_refuses_anything_that_leaves_the_root`、`a_symlinked_working_copy_is_refused`) |
| `cargo test -p nomifun-conversation --lib` | 511 passed(该 crate 已逐字节回到 `HEAD`) |
| `cargo test -p nomifun-auth --lib` | 146 passed |
| `cargo test -p nomifun-app --test miniapp_e2e` | 13 passed(7 条迭代会话用例删除,2 条 `/workspace` 用例新增) |
| `cargo check -p nomifun-app` | 通过(只有既存 dead-code warning) |
| `cargo fmt --check`(五个相关 crate) | 通过 |
| `cd ui && bun run typecheck` | 通过 |
| `cd ui && bun test` | 2057 passed / 0 failed |
| `bun run check` | 九项全绿(i18n 双语对等 5456 键 / 40 模块) |
| `bun run build:ui` | 通过;`ui/dist/nomifun-build.json` 的 `api_contract_version` = 18 = `ui-api-contract-version.txt` |

零残留 grep(`iteration_conversation_id` / `iteration-session` / `iterationSession` /
`miniapp_iteration` / `MiniAppIteration*` / `MINIAPP_*_EXTRA_KEY` / `workspaceNotice`)
在 `crates apps ui/src scripts` 下返回空;仅 `docs/specs/` 的 v2 与本文命中,那是刻意留下的
历史记录。结构测试里的反向断言改用正则字符类(`/workspace[N]otice/`),以免测试自身的
字面量污染这条 grep。

对抗式复核确认(读码 + 上表用例):删除小程序仍会带走 `{work_dir}/miniapps/{id}/`;
`publish` 会拒绝半成品(读前后两次 `stat` 不一致、非 UTF-8、非 HTML 文档三道闸);
`/workspace` 的两条路径(目录与源码文件)都经 `resolve_within_miniapps`,客户端永不传路径;
「继续迭代」建的是 `workspace: ''` + `custom_workspace: false` 的普通托管会话,与小程序无任何
引用关系,删小程序不动它、删它不动小程序。工作区重定向已无任何代码路径:
`nomifun-conversation` 整个 crate 对 "miniapp" 零命中。

尚未跑的两项(需要人工):§7 的端到端 `nomifun-web` 手工走查,以及桌面端手点确认。
