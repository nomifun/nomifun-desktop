# 小程序 v2 — 专属工作空间、迭代会话与导入 · 设计决策

日期:2026-08-10
状态:决策定稿,随码实施
前置:[`2026-08-09-miniapps.zh.md`](2026-08-09-miniapps.zh.md)(v1)。本文**取代** v1 的 D2、D5,
并新增 D8–D14。v1 其余决策(单文件自包含 D1、Nomi 引擎 D4、迭代即继续对话 D6)继续有效。

## 1. 本次要解决的问题

v1 把小程序当作"会话的产物":源码内联进 SQLite,继续改进靠跳回来源会话。实际用起来
三个问题:

1. **来源会话会消失**,跳转是一条会腐烂的链接;而且从 `/mini-apps/:id` 跳
   `/conversation/:id` 会踩到路由级失败(见 §6),用户看到红色错误面板。
2. **没有归属空间**:小程序没有自己的目录,源码、资产、迭代痕迹无处安放。
3. **只能由 AI 生成**:用户自己写好的 H5 无法托管进来。

## 2. 决策

| # | 决策点 | 结论 |
|---|--------|------|
| D8 | 存储分层(**取代 D2**) | **已发布快照留在 SQLite,工作副本落盘**。`miniapps.html` 仍是 `/serve` 直出的内容;每个小程序另有工作目录 `{work_dir}/miniapps/{miniapp_id}/`,内含工作副本 `miniapp.html`,由迭代会话就地编辑 |
| D9 | 工作目录根位置 | `{work_dir}` 而非 `{data_dir}`:这是用户创作中的工作文件,与 `conversations/` 同类。**不注册进 `MANAGED_DATASET_ROOTS`** —— 该注册表已冻结,新增条目需要重铸 `RELEASED_V*_MANAGED_ROOTS` 并 bump `PLAN_VERSION`,历史上已有 `browser-secrets` 回归先例 |
| D10 | 迭代会话(**取代 D5 的标记方案**) | 每个小程序绑定**一个**长期迭代会话:`miniapps.iteration_conversation_id`。由 `POST /api/miniapps/{id}/iteration-session` 幂等置备,照抄 `ensureCompanionSession` 的三个分支(已存在则复用 / 会话已消失则剪除重建 / 注册失败则回删会话) |
| D11 | 会话工作空间形态 | 迭代会话必须以**自定义工作空间**创建(`extra.workspace` 显式给出、不落 `temp_workspace_id`):托管工作空间在会话删除时会 `remove_dir_all`,那会连同小程序源码一起删掉 |
| D12 | 发布动作 | **显式发布**:`POST /api/miniapps/{id}/publish` 读工作副本写入快照。详情页在"工作副本 mtime > 发布时间"时显示「有未发布改动」 |
| D13 | 创建流程(**取代 D5**) | 启动页「创建小程序」先建**草稿小程序行**,置备空间与迭代会话,**明确告知用户会话工作空间将落在小程序专属空间内**,随后落地 `/mini-apps/:id` 并打开迭代面板 |
| D14 | 导入 | v1 支持:单个 `.html` 文件、含 `index.html` 的目录。校验在 **Rust 侧**(同一套规则既回给用户也回给模型);分 FATAL / 可自动修复 / 警告三档;不通过时提供「用会话改造」入口 |

### 为什么保留 SQLite 快照(D8 的理由)

三个都是实测过的具体收益,不是抽象洁癖:

- **`/serve` 永远不会吐出半个文档**。工具链的原子写在 rename 失败时会退化为
  `std::fs::write`,`bash` 的 `> miniapp.html` 截断写根本不原子,而未来 ACP 引擎用的是
  它自己的写入器 —— 谁都不能保证。读快照把这个竞态整类消除。
- **迭代中的破坏不会波及正在用的工具**。用户把应用改坏时,已发布版本照常可用。
- **备份天然覆盖**。数据库快照进备份包,而 `{work_dir}` 下的第二个工作根需要改
  备份清单格式(清单里 `managed_workspaces` 是单值且被校验)。

代价:文档在快照与工作副本各存一份(每个 ≤4 MiB);产品上多出一次发布动作;UI 必须
把"未发布"状态显式呈现,否则用户会报"改了不生效"。工作副本**惰性物化**(首次
继续迭代/导入/发布时 `create_dir_all` + 若缺失则从快照原子落盘),因此换了 `work_dir`
也能自愈。

## 3. 数据模型增量

迁移 `029_miniapp_iteration.sql`(仅加列,append-only;**不 DROP `html`**):

```sql
ALTER TABLE miniapps ADD COLUMN iteration_conversation_id TEXT;  -- 可空 UUIDv7 CHECK
ALTER TABLE miniapps ADD COLUMN published_at INTEGER;            -- ms epoch,可空
CREATE INDEX idx_miniapps_iteration_conversation_id
  ON miniapps(iteration_conversation_id);
```

配套(缺一项**启动即失败**,不是测试失败):`id_schema_contract.rs` 的
`UUIDV7_BUSINESS_COLUMNS` 两处、`LOGICAL_REFERENCES` 里新增
`iteration_conversation_id → conversations`(可空,策略取 SetNull,并在
`sqlite_conversation.rs` 的 SET_NULL 块补一条 `UPDATE miniapps ...`,与既有
`source_conversation_id` 同侧)。`source_conversation_id` **保留**:预览面板的固化
路径仍靠它判断"本会话是否已固化过",只是**不再用于任何跳转**。

## 4. HTTP 契约增量(契约版本 17 → 18)

| 方法/路径 | 语义 |
|---|---|
| `POST /api/miniapps/{id}/iteration-session` | 幂等置备并返回迭代会话 id;顺带 `ensure_workspace` |
| `GET /api/miniapps/{id}/iteration-session` | 只读查询(不存在返回 null,不置备) |
| `POST /api/miniapps/{id}/publish` | 工作副本 → 快照;返回更新后的 `MiniAppResponse` |
| `POST /api/miniapps/import` | 校验 + 导入;失败返回结构化 `MiniAppImportReport` |
| `POST /api/miniapps/validate` | 只校验不导入(导入对话框的即时反馈) |

`MiniAppResponse` 新增 `iteration_conversation_id`、`published_at`、
`has_unpublished_changes`。**响应里仍然不含 HTML 正文**。
`ipcBridge.miniapp-wire.test.ts` 有一条断言 per-app 路由出现次数恰为 3,新增路由需同步。

客户端**永不传路径**:工作目录由后端从 `miniapp_id` 推导,`resolve_within_miniapps`
逃逸守卫(照抄 workshop)在每次打开前调用。

## 5. 导入校验

分档语义:**FATAL** 拒绝导入并给出修改指引;**可自动修复** 导入时由 NomiFun 处理并告知;
**警告** 不阻塞、只提示(沙箱内 `localStorage` 可能抛错属于此档)。校验规则清单、
每条的中英文修复文案、以及不通过时「用会话改造」的提示词与首条消息形状,由实现阶段
按 `/tmp/mini2-import-rules.md` 的调研结论落地并写入本文附录。

关键约束(决定了规则集):文档运行在 CSP `sandbox` 且**无** `allow-same-origin` 的
opaque origin 里 —— 没有 cookie、不能同源 fetch、存储 API 可能抛错、Service Worker
不可用、`window.parent` 不可达;没有构建步骤;没有服务端。

## 6. 已查明:那条被删除的跳转为何报错

日志证据:点击后**全程没有** `GET /api/conversations/{id}`,即路由从未完成首次渲染;
同一按钮在 3 小时前的另一次应用会话里是正常的。因此不是 id 或权限问题,而是
`React.lazy` 动态 chunk 的**加载/求值失败**被路由错误边界接住(红色面板)。本次会话
反复重启 dev server 会使窗口里缓存的 chunk URL 失效,足以解释。

**对新功能的直接约束**:迭代面板 import 的是同一套 `pages/conversation/**` 模块图,
因此必须包一层**可重试的错误边界**,而不是让它裸奔。

## 7. 前端形态

- 详情页 `/mini-apps/:id` 变为两栏:左 `MiniAppFrame`(不变),右 `ContentAside` +
  `useResizableSplit({ unit:'px', defaultWidth:420, minWidth:340, maxWidth:720 })`,
  宽度持久化。移动端панель**替换**而非并排。
- 迭代聊天**必须**独立成 `MiniAppIterationPanel.tsx`:`RunnerPage.tsx` 有一条结构测试
  断言其源码不含 `/conversation/`(这正是记录已删除跳转的那道闸),任何会话模块 import
  都会触发它。
- 面板内:自挂 `PreviewProvider`(`subscribeGlobalOpen={false}`,否则会抢会话页的全局
  `preview.open` 事件)包裹可写 `NomiChat`。缺 `PreviewProvider` 是**直接崩溃**而非降级。
- 迭代会话标记为 `extra.miniapp_iteration: true`(**不是** `extra.miniapp`),否则
  `useAutoPreviewMiniApp` 会在面板内每回合触发、往没有 PreviewPanel 的 provider 里
  开标签。同时需在会话列表过滤中隐藏,避免用户把它删掉造成悬空链接 —— 但即便悬空,
  置备接口也必须能自愈。
- 库页面新增次级「导入小程序」按钮;卡片与只读快捷面板的其余部分不动
  (`updated_at` 已经就是"最近迭代时间")。

## 8. 遗留清理

`miniApps.actions.openSource` 两侧语言包里已成为无引用死键(`check:i18n` 只校验对称性、
不检测未使用),随本次一并删除。同理删除 `miniApps.composer.conversationName`:迭代会话
的标题改为服务端生成(`iteration_conversation_name`),这个键在本次改动中失去了唯一读者。

## 9. 评审后收敛的存储规则

对抗评审发现两层存储在几个边界上会静默丢用户的东西,已在本次修掉,记在这里因为它们是
规则而不是实现细节:

1. **`published_at` 只由"写了 body 的那次写"来盖章。** 原先 `update` 无论是否带 `html`
   都传 `published_at: None`。但 v1 预览面板的「固化为小程序」→「更新」正是经 `update`
   写快照的,于是快照更新了、`published_at` 还停在旧值,详情页继续显示「有未发布改动」,
   一点「发布」就用更旧的工作副本盖掉刚固化的文档。现在:带 `html` 的 `update` 即盖章,
   并把工作副本一起刷成同一份文档,两层不再各写各的。
2. **`publish` 盖的是它读到的那些字节的 mtime,不是 `now_ms()`。** 晚于内容的时间戳会把
   "读取期间落地的那次写"标记为已发布,用户最新的改动从此既不上线也不再出现「发布」按钮。
3. **`publish` 前后各 `stat` 一次,并复用 import 的文档形状校验。** 没有任何东西给工作副本
   的写入排序(`bash` 的 `> miniapp.html` 先截断再写),所以读取横跨一次写入时必须拒绝而不是
   把半个文档提升为线上版本;同理,非空 UTF-8 不等于"是个网页",计划稿/报错文本也不允许覆盖
   正在用的工具。
4. **工作副本存在但 `published_at` 为 NULL 一律算未发布。** 原先回退到 `updated_at`,而改名
   会推动 `updated_at`,于是"改个名字"就能让真实的未发布改动连「发布」入口一起消失。
5. **删除小程序会级联删除它的迭代会话与 `{work_dir}/miniapps/{id}/`。** 这棵树故意不在
   `MANAGED_DATASET_ROOTS` 里,没有任何其他清理会碰它;会话则会留下一个绑在已死小程序上、
   产出永远无法发布的活会话。
6. **注册迭代会话改为条件写入**(`WHERE iteration_conversation_id IS NULL`)。两个客户端
   同时首开同一个小程序时,原先的后写覆盖会留下一个无人引用、却仍绑着真实工作目录的活会话。

## 10. 后续项(本次不做)

- **发布没有撤销,也看不到工作副本。**「发布」是唯一能看到 AI 产出的方式,而它会覆盖唯一
  的已发布快照。要真正解决需要快照历史(至少保留上一版 + 一步回滚),或者一个能预览工作
  副本的surface —— 都要新的迁移或新的路由,大于本次改动。当前的收敛是让 `publish` 拒绝
  半个文档和非文档内容(见 §9.3),即"不会悄悄变坏",但"变坏之后能退回去"仍然缺失。
- **`published_at` 语义被 `ensure_workspace` 借用。** 它现在既表示"快照何时成为当前版本",
  也表示"工作副本何时与快照一致";前端因此无法用 `published_at == null` 判断草稿,改用
  占位文档的精确字节数(常量与文档同源,见 `contract.ts`)。彻底修法是加一列
  `working_copy_synced_at`,属于迁移级改动。
- **导入不问名字/描述/图标。** 名字取 `<title>`,没有 `<title>` 时退到文件名(本次修掉了
  超长 `<title>` 会在 `create` 处报错的问题)。让导入报告那一步带一个名字输入框是更好的
  产品答案。
- **`fsio.rs` 是 `nomifun-workshop/src/fsio.rs` 的逐字副本。** 原子写原语应当和
  `miniapp_workspace.rs` 一样下沉到 `nomifun-common`,否则对 temp+rename 方案的修正只会
  落到其中一份上。跨 crate 重构,单独做。
- **构建提示词在 TS 与 Rust 各存一份,没有校验闩。** `contract.ts` 的
  `MINI_APP_BUILDER_SYSTEM_PROMPT` 与 `prompt.rs` 的 `iteration_system_prompt` 前六条规则
  逐字相同,改一边不会让另一边失败,两条创建路径会教给模型不同的契约。
- **「用会话改造」把用户真实项目的绝对路径交给一个文件权限不受限的会话。** 这不是本次新增
  的能力:桌面 surface 上每个会话的 `file_authority` 都是 `PathAuthority::Unrestricted`
  (`caps_files.rs`),`POST /api/fs/read` 也早已如此。本次只把只读约束从提示词第 5 条提到
  最前面并强化措辞。真正的解法是按会话收紧文件权限(只读 + 限定在所选目录),那是一个平台
  级能力。
