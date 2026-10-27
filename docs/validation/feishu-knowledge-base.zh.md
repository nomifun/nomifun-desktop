# 飞书知识库（远端实时检索型）验收记录

对应[现役实施计划](../handoffs/feishu-knowledge-base.zh.md)。验证日期：2026-10-27；工作区 HEAD：`7b288861` + 本专项改动。

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
提供远端召回。知识链路经 office-hub 内部 `knowledge_*` 方法，受 `ready()` + scope
检查与 ≤4 并发信号量约束。

## 分层落地对照

| 层 | 实现 |
| --- | --- |
| `nomifun-db` | `KnowledgeSourceKind::Feishu`；020 前向迁移把 `kind` CHECK 扩为 `('url','feishu')` 并做 FK 完整性证明 |
| `nomifun-api-types` | `KnowledgeMountInfo.remote_index_stale`（serde-default 兼容） |
| `nomifun-office-hub` | `documents.read`/`documents.inspect` capability；`knowledge_search_documents`/`knowledge_fetch_document`/`knowledge_resolve_link` 内部方法；Search v2/fetch/inspect typed 命令与解析器 |
| `nomifun-knowledge` | `FeishuKnowledgePort` + fail-closed 默认；stub 投影（标题命名 + name_hint）；策展链接校验（官方域/doc|docx|wiki/≤64）；远端检索∩策展集；`.remote/feishu/` 伪路径 handle 与实时读取；远端正文写拒绝；挂载组装点索引刷新 + stale 标记；typed binding 冻结策展 token 集 |
| `nomifun-app` | 组合根 `OfficeFeishuKnowledgePort::new(office_hub)` 注入 `KnowledgeService` |
| UI | CreateStudio 第三方组「飞书文档」（链接可留空）；详情页「添加知识 → 飞书文档」（仅 feishu 库）；工作台行级「存入知识库」；kind 徽标/筛选/双语；`IKnowledgeBase.kind` 增 `'feishu'`；addContent `feishu` 变体；UI/API 合同 51 |

## 自动验证（已执行）

| 检查 | 结果 |
| --- | --- |
| `cargo test -p nomifun-office-hub` | 72 passed，4 ignored（需真实 CLI/profile 的探针） |
| `cargo test -p nomifun-knowledge --lib` | 365 passed（含 feishu 链接校验、stub 投影、策展集交集检索、实时读取、写拒绝、挂载 stale） |
| `cargo check -p nomifun-app` | 通过 |
| `bun test --cwd ui src/renderer/pages/knowledge/` | 158 passed |
| `bun run typecheck` | 通过 |
| `bun run check:i18n` | 通过（8459 键已重新生成） |
| `bun run check:desktop-ui-boundary` | 通过 |
| `bun run check:agent-session-boundary` | 通过（020 迁移登记进显式迁移清单并附内容规则：仅允许重建 `knowledge_sources`） |

## 待验（真实租户）

- 端到端真实验收：已连接飞书连接器（`nomifun-office-2` profile，user token 有效）条件下
  创建飞书库 → 添加文档链接 → 挂载进会话 → `knowledge_search`/`knowledge_read` 全链路。
  真实 CLI 的 search/fetch 响应结构已实测（§13），但完整知识链路未在真实租户跑过。
- `drive +inspect` 的 `drive:drive.metadata:readonly` scope 在现有 profile 上缺
  （missing_scope 已实测分类正确）；未授权时索引刷新按设计降级并标 stale。

## 边界确认

- 正文不落本地库（唯一例外：既有 `copy_as_editable` 显式动作）。
- 工作台 UI 无正文预览/编辑；`documents.read`/`documents.inspect` 仅知识链路——已在
  `office-feishu-workbench.zh.md` §3 登记合同修订。
- `.remote/feishu` 伪路径写拒绝；一期不支持 sheet/bitable/live 模式/账号全域库。
