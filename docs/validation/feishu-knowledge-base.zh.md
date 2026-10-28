# 飞书知识库（远端实时检索型）验收记录

对应[现役实施计划](../handoffs/feishu-knowledge-base.zh.md)。验证日期：2026-10-27；首轮基线 `7b288861`，
picker 串联修复基线 `8b5e7a5e7` + 工作区改动。

## 目标形态

飞书知识库 = 普通 `knowledge_bases` 注册行 + `extra.source` 中 `kind="feishu"` 的远端源 +
一组策展 `knowledge_source_items`。库根目录存**元数据 stub**（文档标题命名、含稳定
`kdoc_` handle 与 `knowledge_read` 指引）；正文不落本地。挂载即挂载选定文档集合：
junction/TOC/绑定零改动；`knowledge_search` 走 Search v2 并与策展 token 集求交，
`knowledge_read` 经官方 lark-cli 实时取正文。挂载时索引刷新有界（≤5s 等待），失败降级为
上次索引并在 `KnowledgeMountInfo.remote_index_stale` 标记。

## 已核定命令面

见实施计划 §13（lark-cli 1.0.98 真实调用验证）。要点：`docs +fetch` 直接接受 wiki
URL 并自动解包；`drive +inspect` 提供索引刷新的元数据；`drive +search`（Search v2）
提供远端召回。知识链路经 飞书连接器内部 `knowledge_*` 方法，受 `ready()` + scope
检查与 ≤4 并发信号量约束。

## 分层落地对照

| 层 | 实现 |
| --- | --- |
| `nomifun-db` | `KnowledgeSourceKind::Feishu`；020 前向迁移把 `kind` CHECK 扩为 `('url','feishu')` 并做 FK 完整性证明 |
| `nomifun-api-types` | `KnowledgeMountInfo.remote_index_stale`（serde-default 兼容） |
| `nomifun-feishu-connector` | `documents.read`/`documents.inspect` capability；`knowledge_search_documents`/`knowledge_fetch_document`/`knowledge_resolve_link` 内部方法；Search v2/fetch/inspect typed 命令与解析器 |
| `nomifun-knowledge` | `FeishuKnowledgePort` + fail-closed 默认；stub 投影（标题命名 + name_hint，默认落库根而非 `snapshots/`，可按 `destination_parent_path/id` 落到选中文件夹）；策展链接校验（官方域/doc|docx|wiki/≤64）；空策展集是合法飞书源——建库、移除最后一篇都不剥离 `extra.source`；启动时为被旧缺陷剥离缓存的库按唯一 active 飞书源重建缓存；picker 标题规范化（去 NUL、≤1024 字节）；远端检索∩策展集；`.remote/feishu/` 伪路径 handle 与实时读取；远端正文写拒绝；挂载组装点索引刷新 + stale 标记；typed binding 冻结策展 token 集 |
| `nomifun-app` | 组合根 `FeishuConnectorKnowledgePort::new(feishu_connector)` 注入 `KnowledgeService` |
| UI | CreateStudio 第三方组「飞书文档」（创建只填基本信息，文档创建后策展）；飞书库详情页右侧默认即 `FeishuDocPickerPanel`（替换预览区，树自动选中首个文件或加入文档后都不切回预览）：搜索/分页浏览账号飞书文档、一键加入当前选中目录（标注「加入到：…」），加入后左侧目录刷新并定位新 stub；点击树中文件才进入预览，工具栏「飞书文档」返回；「添加知识」菜单提供「选择飞书文档」与「粘贴飞书链接」；工作台行级「存入知识库」；kind 徽标/筛选/双语；addContent `feishu` 变体 `{docs:[{url,title?}], destination_parent_path?, destination_parent_id?}`——picker 标题使首次发布免调 `drive +inspect`；UI/API 合同 52 |

## 自动验证（已执行）

| 检查 | 结果 |
| --- | --- |
| `cargo test -p nomifun-feishu-connector` | 72 passed，4 ignored（需真实 CLI/profile 的探针；首轮执行，本次未改 feishu-connector） |
| `cargo test -p nomifun-knowledge --lib` | 373 passed（feishu 子集 16：链接校验、空策展集建库后 kind/source 保持、picker 标题免 inspect、移除最后一篇仍可策展、启动修复被剥离的缓存、按选中文件夹落位、超长标题规范化、stub 投影、策展集交集检索、实时读取、写拒绝、挂载 stale；路由级 `feishu_add_content_route_round_trips_picker_docs` 覆盖 HTTP 线形、重复、文件夹落位、无标题 fail-closed、旧 `urls` 线形 400） |
| `cargo check -p nomifun-app` | 通过 |
| `bun test --cwd ui src/renderer/pages/knowledge/` | 173 passed（`FeishuDocPickerPanel` 10 例：搜索/分页/一键加入/kind 过滤/已策展/缺 browse scope 门控/未连接引导/重复/落位字段/索引失败单一告警；`FeishuKnowledgeDetail` 4 例渲染真实详情页：有文件时默认即 picker、加入后保持 picker 并在目录定位新 stub、点文件进预览且可返回、空白库无 picker） |
| `bun run typecheck` | 通过 |
| `bun run check:i18n` | 通过（8475 键已重新生成） |
| `bun run check:desktop-ui-boundary` | 通过 |
| `bun run check:agent-session-boundary` | 首轮通过（020 迁移登记进显式迁移清单并附内容规则：仅允许重建 `knowledge_sources`）；本次未改 Session/迁移 |

## 真实租户端到端（2026-10-27 已执行）

隔离环境：`nomifun-web --api-only --insecure-no-auth`，全新 `NOMIFUN_DATA_DIR`（临时目录，未触碰
用户数据目录），共享 lark-cli 1.0.98，profile `nomifun-office-2`，飞书侧仅只读（Search v2、docs fetch）；
只策展标题含 `nomifun-test` 的专用测试文档（wiki 节点），正文仅记录长度与首行。15/15 通过：

- 连接器 ready，`documents.browse`/`documents.read` 已授权；picker 同款 飞书连接器查询返回 3 篇测试文档。
- 建空飞书库返回并复读均为 `kind="feishu"`、`source.entries=[]`（修复前此处即变为 blank）。
- 带 picker 标题一键加入：`added=1, indexed=1, failed=0`，stub 落库根；再次加入 `duplicates=1`；
  `source.entries` 含该文档（`syncStatus=synced`），目录树列出 stub，stub 仅含元数据与远端标识。
- `.remote/feishu/<token>` 经 `docs +fetch` 实时读到正文（wiki URL 自动解包）；`/api/knowledge/search`
  同时返回远端命中 `.remote/feishu/<token>` 与本地 stub。
- 移除最后一篇后仍为 `kind="feishu"`、条目为空；再次加入重新索引成功；测试库已删除。

环境备注：该 Windows 调试构建的 `nomifun-web.exe` 在 “module state build” 阶段主线程（默认 1 MB）
栈溢出，E2E 以 32 MB 主线程栈重链接该 exe 后运行；本次改动未触及 `nomifun-app`/`nomifun-web`，
是否为既有问题未单独追查。

## 待验

- 挂载进真实 Agent 会话后由模型调用 `knowledge_search`/`knowledge_read`：上面验证的是 HTTP 层
  search/read；Agent 绑定路径（`BoundKnowledgeReadService`）只有单测覆盖，模型侧真实调用未跑。
- `drive +inspect` 的 `drive:drive.metadata:readonly` scope 在现有 profile 上缺
  （missing_scope 已实测分类正确）；picker 标题免 inspect 的首发路径已覆盖单测与真实 E2E，
  挂载刷新仍走 inspect，未授权/远端失败按设计降级并标 stale——last-good stub 保留，不静默重发。

## 本次修复（picker 串联）记录

- **根因（已在用户 dev 数据中确认）**：空策展集飞书库在 `refresh_legacy_source_cache`
  中 items 为空即被 `clear_legacy_source_cache` 抹掉 `extra.source`——建库后立即丢失
  `kind="feishu"`（显示为空白库），详情页没有飞书入口、工作台「存入知识库」列不出该库、
  `append_feishu_entries` 以 "knowledge base has no Feishu source" 拒绝，即"添加是假的"。
  用户 dev 库中两个飞书库均为：normalized 飞书源 active、0 条目、`extra.source` 缺失。
  修复：Removed 状态才清理缓存，空飞书策展集作为合法源持久化；移除最后一篇同样保留
  飞书源；启动修复为已被剥离的库重建缓存（仅当该库唯一 active 源为飞书源）。
- **产品面**：详情页原本只有手贴链接弹窗，且树自动选中首个文件会让右侧回到预览；现
  飞书库右侧默认是搜索/浏览/一键加入面板，加入后保持面板并在目录中定位新 stub，
  stub 落库根或选中文件夹；工作台「存入知识库」与粘贴链接作为补充路径保留。
- **回归修正**：inspect 失败不得回落已存标题重发 stub——否则挂载刷新永远报成功，
  `remote_index_stale` 永不置位（已删回退并恢复 stale 语义）。
- **已知限制**：飞书连接器把 Search v2 的 wiki 结果统一归为 `wiki`（不区分底层对象类型），
  因此底层为表格/多维表格的 wiki 节点也会出现在 picker 中并可加入；一期正文读取走
  `docs +fetch`，这类节点的实时读取行为未验证。

## 边界确认

- 正文不落本地库（唯一例外：既有 `copy_as_editable` 显式动作）。
- 工作台 UI 无正文预览/编辑；`documents.read`/`documents.inspect` 仅知识链路——已在
  `feishu-connector.zh.md` §3 登记合同修订。
- `.remote/feishu` 伪路径写拒绝；一期不支持 sheet/bitable/live 模式/账号全域库。
