# 飞书知识库：远端实时检索型知识库实施方案

日期：2026-10-27。状态：已批准设计，待实施。基线：main `7b2888610c95c9a6d786faffb065ab5545a8c848`；后续接续以实际 HEAD 为准。

本文件是本专项唯一现役施工与验收说明。启动入口见[启动 prompt](feishu-knowledge-base.start-prompt.zh.md)。实施前阅读根目录 [AGENTS.md](../../AGENTS.md)、[办公飞书计划](office-feishu-workbench.zh.md)（本专项登记其合同修订，见 §2-F07）、[Agent Session](../architecture/agent-session.zh.md) 与[开发验证](../contributing/development.md)。

**2026-10-27 用户决策依据**：同步模型选择**远端实时检索**（正文不落本地库）；一期文档类型仅 doc/docx + wiki 解析；批准新增 `documents.read` 能力但**仅限知识链路**（工作台 UI 仍不做正文预览/编辑）；新鲜度策略为**挂载时自动刷新索引**。产品形态：飞书知识库 = 本地一个文件夹（即一个知识库），其中"移入"若干选定的飞书文档名称作为索引；挂载知识库即挂载该选定文档集合，交互与现有知识库完全对齐。

---

## 1. 目标与交付边界

1. **飞书知识库 = 策展文档集**：用户可创建 N 个飞书知识库；每个库是一个本地目录，内含若干以文档标题命名的索引条目（stub）。选中、移入、移出与现有知识库条目管理交互一致。
2. **挂载零新增机制**：飞书库复用既有 junction/symlink 挂载、绑定选择器、TOC 注入、prompt 合同、MCP 暴露；agent 看到的挂载目录中是按名称排列的文档索引。
3. **检索远端实时**：`knowledge_search` 经飞书 Search v2 召回并与策展集求交；`knowledge_read` 经 CLI 实时取正文。正文不写入本地库目录。
4. **挂载时自动刷新索引**：会话挂载前对飞书库做有界索引刷新（元数据级），目录投影随之更新；失败降级为上次索引并明确标注过期。
5. **合同收敛**：`documents.read` 只服务知识库链路；办公工作台文档正文"浏览器打开"合同不变，仅在本文件登记该修订（office-feishu-workbench.zh.md §3 的"正文不落库"限定于工作台 UI，知识链路经由新能力读取）。

**本版本不做**（见 §10）：bitable/sheet/mindnote/slides 正文、账号全域检索库、定时刷新、写回飞书、live 模式源、工作台正文预览/编辑、正文快照落库、stub outline 注入。

## 2. 强制约束

| 编号 | 约束 | 执行要求 |
| --- | --- | --- |
| F01 | 策展集即语料边界 | 每个飞书库的检索语料 = 该库 `knowledge_source_items` 的 token 集；远端搜索结果必须与之求交；一期不提供"账号全域"库形态。 |
| F02 | 正文不落库 | 远端读取只进内存与 agent 上下文；stub 只含元数据与指引，不含正文。唯一例外是用户显式 `copy-as-editable`（D10）。 |
| F03 | 挂载契约不变 | `.nomi/knowledge` junction、绑定类型、TOC 预算、写回语义、MCP 暴露全部复用；不新增第二套挂载/绑定机制。 |
| F04 | 唯一事实链 | 复用 `knowledge_sources`/`knowledge_source_items`/`knowledge_entry_provenance`/`knowledge_entries` 投影；不新增平行账本；AgentSession/Turn/Event/Payload/Effect 不动。 |
| F05 | 端口化依赖方向 | `nomifun-knowledge` 只定义 `FeishuKnowledgePort` trait 与 fail-closed 默认实现；`nomifun-app` 组合根注入 office-hub 实现。knowledge crate 不依赖 office-hub。 |
| F06 | 失败降级可解释 | 索引刷新失败不阻塞会话启动；检索/读取远端失败返回可操作错误（含"连接失效请重新授权"类指引）；stale 索引进 context 标注。 |
| F07 | 权限按用申请 | `documents.read` 新 capability 仅服务知识链路；缺 scope 走既有按需授权流；scope 集合以已安装 CLI 预检与官方文档核定后登记进 `adapter::permissions`/`allowed_scopes`。 |
| F08 | 一期类型边界 | 只接受 doc/docx 文档链接与 wiki 节点链接（wiki 解析为底层文档）；sheet/bitable/mindnote/slides/file 在选择与建库时明确拒绝并提示后续支持。 |
| F09 | CLI 纪律 | 沿用 typed JSON 命令边界；分页 token 原值透传；不拼接未核验 URL；错误分类沿用 `output_error`/`auth_output_error`。 |
| F10 | UI 边界 | 桌面 880×600 及以上；跑 `bun run check:desktop-ui-boundary`；不为远端库做移动端分支。 |

## 3. 已定技术决策

实施中不再重新讨论；只有必须改变产品合同或出现无法推断的关键事实时才提出问题。

| 编号 | 决策 |
| --- | --- |
| D01 | **源模型**：新增 `KnowledgeSourceKind::Feishu => "feishu"`（persisted enum）。飞书库在 `extra.source` 挂单个 `kind: "feishu"` 源；`mode` 仅接受 `snapshot`（语义重定义为"索引投影"，不是正文快照）；`live` 在 create/set_source 时拒绝。 |
| D02 | **条目身份**：`item.requested_url`/`normalized_url` 存规范化飞书链接（`official_url` + 路径形态校验：`/{docs,docx,wiki}/<token>`）；`item.title` 存文档名；`item.etag` 存远端 `update_time`；`final_url` 存 wiki→docx 解析后的落地链接；`sync_status` 复用为索引态（synced=在索引中、failed=拉取失败、missing=远端删除或失权）。 |
| D03 | **stub 投影**：每个 active item 对应库根目录（或 `default_parent_entry_id` 指定子目录）下一个真实 `.md` 文件，文件名 = 清洗后文档标题（允许 CJK，剥离路径非法字符），重名按 `标题 (2).md` 序列分配；远端改名在索引刷新时走 entry rename 机制跟随。内容 = 元数据 quote 块（`source_url`、`final_url`、`nomifun_source_item_id`、`nomifun_source_relationship: managed`、`title`、`remote_updated_at`、`index_refreshed_at`）+ 该文档的稳定 `kdoc_` handle + 一句 agent 指引（"call knowledge_read with this handle for live content"）。provenance=managed：可整理移动、正文只读。 |
| D04 | **稳定 handle**：`encode_doc_handle(kb_id, ".remote/feishu/<norm-token>")`，伪路径只含规范化请求 token，不随 stub 改名失效。`.remote/` 为保留虚拟命名空间：`safe_md_path` 拒绝该前缀的物理读写；远端 read 路径（`knowledge_read`/MCP）解出 token 后查 item 行取 `final_url` token 再抓取；item 不存在时返回明确失效错误。 |
| D05 | **检索分发**：库的远端属性由 `extra.source.kind == "feishu"` 派生，不单列字段、不双写。`BoundKnowledgeBase` 增加 remote 描述（随 typed binding 冻结进 `AgentKnowledgeResource`，`agent_resource_identity` 扩展返回后端标记，身份校验照旧）。`BoundKnowledgeReadService` 增加可选 remote port；search 命中 feishu 库 → 远端 `drive +search --doc-types doc,docx,wiki` ∩ 策展 token 集，并 ∪ 本地 stub 关键词兜底（兜飞书索引延迟），按 token 去重，远端结果优先排序；read 命中 `.remote/` → port.read。route 级 `POST /api/knowledge/search` 走同一分发。 |
| D06 | **挂载时索引刷新**：在 mounts 组装点（`service.rs` mounts_for_* 与 `mount_info_for_bases` 的 TOC 计算之前）对每个含 feishu 源的库启动索引刷新：逐 item 校验可达性与元数据（标题/update_time），更新 item 行、改/建/清 stub（孤儿清理复用 `prune_orphan_snapshots`/`allocate_source_trash_path` 语义）。刷新等待有小超时（默认 5s）：及时完成则 TOC 用新索引；超时先挂载旧索引、后台继续、`KnowledgeMountInfo` 增加 `#[serde(default)]` optional 的 stale 标注字段（context.rs 渲染为一行说明）。索引刷新并发与全局限额沿用既有 source 并发原语 + port 内建信号量（≤4）。 |
| D07 | **office-hub 出口**：adapter 新增 `docs +fetch`（doc/docx 正文）、wiki 节点解析（wiki token → obj_token）与元数据读取（按已安装 CLI `--help` 核定：优先 meta-only 命令，缺失时用受控轻量 fetch）的类型化封装；`permissions()` 增加 `documents.read` capability；service 暴露内部方法 `search_documents`/`fetch_document`/`resolve_link`（同进程调用，不进工作台 HTTP 合同），走 `ready()`（enabled+profile+runtime+actor）+ scope 检查 + `output_error` 分类。 |
| D08 | **条目上限**：feishu 源独立上限 `MAX_FEISHU_SOURCE_ENTRIES = 64`（每 item 一次远端调用，挂载刷新有界；超过拒绝并提示拆分多个库）。URL 源 `MAX_SOURCE_ENTRIES = 16` 不变。 |
| D09 | **写回语义**：远端条目正文只读（managed）；agent/用户可在该库新建本地 `.md`（editable 树，`user` origin，与飞书索引并存）；不回推飞书。`knowledge_write` 对 `.remote/` handle 返回明确拒绝。 |
| D10 | **copy-as-editable**：对 feishu 条目 = 一次 `docs +fetch` 取正文落本地副本（provenance=copy），复用现有 `copy_entry_as_editable` 管线，正文获取经 port。detach/remove-source 语义照旧（stub 进 `.nomifun-trash`）。 |
| D11 | **UI 形态**：CreateStudio 恢复 `feishu` 类型（连接未就绪 → 引导连接器设置；就绪 → 建库后进入详情页选文档）；详情页源区域列出条目 + 索引状态 + 逐条刷新/浏览器打开/移出/copy-as-editable；`OfficeQuery::Documents` 复用为多选 picker + 粘贴链接（`official_url` 校验）；`/knowledge/feishu` 工作台每条文档加"存入知识库"（选目标飞书库 → append source entry，走 `PUT /bases/{id}/source`）。`derive_kind` 对 feishu 源返回 `"feishu"`；列表卡片加飞书徽标。 |
| D12 | **观测**：条目 `sync_status`/`last_error`/`etag` 展示于详情页；库级 stale/连接状态在详情页与挂载 context 各有一处明确标注。`resume_pending_source_fetches` 的 kind 过滤扩展到 feishu（`service.rs:2313`）。 |

## 4. 现状锚点

| 事实 | 位置 |
| --- | --- |
| 源 kind 白名单拒绝点（`!= "url"`） | `crates/backend/nomifun-knowledge/src/service.rs:9810` |
| UI 库类型派生（`"web"`/`"blank"`/`"local"`） | 同文件 `derive_kind` ~9793 |
| 启动恢复 kind 过滤（`mode==Snapshot && kind=="url"`） | 同文件 ~2313 |
| URL 源抓取→托管发布管线（状态机/staging/孤儿清理） | `prepare_managed_source_items` ~7068；`publish_managed_source_items` ~7680；`recover_pending_source_publications` ~7170 |
| 托管快照头与 `nomifun_source_item_id` 标记 | `src/source_url.rs:363/421` |
| 端口注入先例（render port + fail-closed 默认） | `source_url.rs:75-105`；`service.rs:756/898/1945`；`nomifun-app/src/router/state.rs:506` |
| 挂载组装与 TOC | `service.rs` ~8690（绑定路径）/`mount_info_for_bases` ~8724（AgentSession typed 路径） |
| `kdoc_` handle 编解码 | `service.rs:123-144` |
| agent 侧授权读取服务 | `src/agent_module.rs:311`（`AuthorizedKnowledgeService`）；`src/service/bound.rs:81`（`BoundKnowledgeReadService`） |
| 源/条目管理路由 | `src/routes.rs:44-103`（`set_source`/`refresh-source`/`refresh_entry_source`/`copy-as-editable`/`detach-source`/`remove-source`） |
| 办公侧命令边界 | `nomifun-office-hub/src/adapter.rs`：`permissions` 93、`query_commands` 406、`link_target`/`official_url` 183/204；`service.rs`：`ready()` 213、`query` 228 |
| 办公合同"正文不落库"（本专项修订，见 F07） | `docs/handoffs/office-feishu-workbench.zh.md` §3（约 108-110 行） |
| app 组合根 | `nomifun-app/src/services.rs:2064-2127`（KnowledgeService 构建/仓注入/completer/MCP） |
| UI 断言"feishu 非源类型"（随本计划退役） | `ui/src/renderer/pages/knowledge/feishuKnowledgeEntry.test.ts` |
| lark-cli 上游能力参考（`docs +fetch`、wiki、分页） | `nomifun-skill-library/assets/builtin-skills/lark-cli/references/commands.md` |

## 5. 目标架构

```text
建库/策展                              挂载(会话开始)                     使用中检索
─────────                              ─────────────                     ─────────────
CreateStudio/详情页                     mounts_for_* /                      knowledge_search
  └─ createBase/set_source              mount_info_for_bases                ├─ 本地库 → 现管线
     entries[].url=feishu 链接             ├─ 索引刷新任务(≤5s 等待)         └─ feishu 库 → remote port
        │                                  │   drive+search meta /          │   ├─ drive +search
        ▼                                  │   docs +fetch --meta           │   │   ∩ item token 集
  KnowledgeSourceItem                    │   → item/stub 更新/清理          │   ├─ ∪ stub 本地关键词兜底
  (requested_url=feishu 链接)            ▼                                  │   └─ hits + kdoc_ handle
        │                              stub .md 投影(标题命名)              ▼
        ▼                                  │                              knowledge_read(handle)
  索引刷新→stub 生成                       ▼                                  └─ .remote/ → token→item→
  (provenance=managed)                 junction → .nomi/knowledge/<name>        docs +fetch 实时正文
                                           + TOC(文档名) + stale 标注
```

依赖方向：`nomifun-knowledge` 定义 `FeishuKnowledgePort`（search/read/resolve/metadata）+ `UnavailableFeishuKnowledgePort` fail-closed；`nomifun-app` 注入 `OfficeFeishuKnowledgePort`（内持 `Arc<OfficeHubService>`，实例 owner 绑定，不暴露跨 owner 选择）。

## 6. 分层改动

| 层 | 改动 |
| --- | --- |
| `nomifun-api-types` | `KnowledgeSource` 接受 `kind:"feishu"`；条目 DTO 复用 `KnowledgeSourceEntry`（`url`=飞书链接、`title`=文档名、`rendered` 对 feishu 拒绝）；`KnowledgeMountInfo` 增 optional stale 字段（`#[serde(default)]`）；`derive_kind` 新值 `"feishu"` 的 TS 绑定。 |
| `nomifun-db` | `KnowledgeSourceKind::Feishu` persisted enum；无新表。 |
| `nomifun-office-hub` | adapter：`docs +fetch`、wiki 解析、元数据读取命令与解析器；`documents.read` capability + scope 登记；service：`search_documents`/`fetch_document`/`resolve_link` 内部方法。 |
| `nomifun-knowledge` | `FeishuKnowledgePort` trait + 默认 fail-closed；`validate_source` 放行并校验 feishu（链接形态、类型白名单、≤64、禁 live/`rendered`）；`derive_kind`；stub 投影写入/改名/清理；索引刷新服务方法（复用 source 锁、状态机、trash）；挂载组装点挂钩；`BoundKnowledgeBase` remote 标记 + `agent_resource_identity` 扩展；`BoundKnowledgeReadService`/route 搜索远端分发与 `.remote/` read；写策略拒绝远端正文。 |
| `nomifun-app` | 组合根 `set_feishu_knowledge_port(OfficeFeishuKnowledgePort)`；typed binding 解析处透传 remote 标记；MCP server 服务引用不变（同一 KnowledgeService）。 |
| `ui` | CreateStudio `feishu` 类型 + 连接引导；详情页索引条目管理与"添加飞书文档"picker（复用 Documents 查询组件）；工作台"存入知识库"；`feishuKnowledgeEntry.test.ts` 等断言按本决策反转；i18n 双语。 |
| 文档 | 本文件；office-feishu-workbench.zh.md §3 追加一行修订说明（`documents.read` 仅知识链路）；新建 `docs/validation/feishu-knowledge-base.zh.md` 记录验收。 |

## 7. 交互流程

1. 知识库列表 → 新建 →「飞书知识库」：连接未就绪展示引导卡（去连接器设置）；就绪则建库（空策展集）。
2. 库详情页 →「添加飞书文档」→ picker：搜索/浏览（`Documents` 查询）多选，或粘贴完整飞书链接；确认后文档以标题出现在条目树，索引状态随行展示。
3. 条目操作：刷新索引、浏览器打开（`officeResourceUrl`）、移出库（item removed + stub 清理）、copy-as-editable（取正文转本地文档）、整理移动（relocate 照旧）。
4. 会话/Agent 绑定选择器中该库与普通库同列；挂载后 `.nomi/knowledge/<name>/` 下是按标题命名的索引文件，prompt TOC 列出文档名，feishu 库附 stale/远端说明行。
5. agent 侧：`knowledge_search` 返回 hits（含 `kdoc_` handle）→ `knowledge_read` 取实时正文；直接 Read stub 文件也会看到元数据与 handle 指引。
6. `/knowledge/feishu` 工作台每条文档「存入知识库」→ 选库 → append entry → 触发该库索引刷新。

## 8. 错误与降级语义

| 场景 | 行为 |
| --- | --- |
| 连接未启用/未登录/运行时缺失（建库、加文档） | 阻断并引导到连接器设置；不产生半成品源。 |
| 索引刷新时连接失效/token 过期 | item 记 `failed`（可操作错误文案）；保留上次 stub；挂载不阻塞，context 标 stale。 |
| 远端文档删除/失权 | item 记 `missing`；stub 保留并标记失效一行；从搜索语料集剔除。 |
| 远端改名 | 索引刷新走 entry rename 跟随；handle 不变（D04）。 |
| `knowledge_search`/`read` 调用时远端不可用 | 工具返回结构化错误（含重新授权指引）；不静默空结果。 |
| 挂载目录物理读写 `.remote/` | `safe_md_path` 拒绝；伪路径只存在于 handle。 |
| CLI 进程并发/超时 | port 内建信号量 ≤4、单次超时（meta 10s / search 20s / read 30s，实施时按 CLI 实测收敛）；超限逐条记 failed，不拖垮整库刷新。 |

## 9. 安全与权限

- 凭据/token 只留官方 CLI store；正文经 CLI → 内存 → agent 上下文，不落库（F02）。
- 远端正文是 agent 上下文的不可信输入，与网页快照同级；CLI 输出错误沿用 `safe_message` 脱敏。
- port 使用实例 owner 身份；不接受前端传入的 profile/凭据覆盖；SSR­F 面不变（CLI 直连飞书，不经 `HttpFetcher`）。
- 飞书链接校验收敛到官方域（复用 `official_url`/`link_target` 同等规则：feishu.cn/larksuite.com/larkoffice.com 的 HTTPS 链接）。

## 10. 不做清单（本版本）

- bitable/sheet/mindnote/slides/file 正文导出（选择与建库时明确拒绝）。
- 账号全域检索库（无策展边界形态）。
- 定时/周期刷新、stub outline 注入、本地正文快照选项。
- 写回飞书（`docs +create`/正文更新）；工作台 UI 正文预览/编辑（合同不变）。
- 远端库的本地 embedding 索引（无正文可嵌）。
- `live` 模式 feishu 源、移动端布局。

## 11. 验证

- Rust 单测：office-hub adapter 新命令/解析（mock `CliOutput`）；knowledge：feishu 源校验与拒绝路径、item 生命周期、stub 投影与改名、孤儿清理、索引刷新成功/失败/超时降级、search∩策展集 + stub 兜底去重、`.remote/` handle 编解码与 read 分发、远端正文写拒绝、copy-as-editable、resume_pending 覆盖。
- 集成：nomifun-app port 接线、typed binding remote 冻结、MCP 暴露路径。
- UI：CreateStudio/picker/详情页/工作台入口交互测试；退役断言更新。
- 边界：`bun run check:desktop-ui-boundary`、`bun run check:agent-session-boundary`、受影响 canonical 测试。
- 真实飞书租户验收（登记到 `docs/validation/feishu-knowledge-base.zh.md`）：连接就绪 → 建库 → 选 N 文档 → 挂载会话 → search 命中 → read 实时正文 → 远端改名/删除/失权的索引语义 → token 过期降级 → 断开连接后的 stale 行为。自动化、真实 CLI、未执行项分别记录，未验不宣称支持。

## 12. 执行进展（接续点）

实施者在此节追加进展：源码/构建身份、实际命令、非零测试数、真实飞书证据、待验原因与下一动作。

| 日期 | HEAD | 阶段 | 证据/命令 | 结果 | 下一动作 |
| --- | --- | --- | --- | --- | --- |
| 2026-10-27 | `7b288861` | 计划定稿 | 本文件 + start-prompt | 设计批准 | S1 |
| 2026-10-27 | `7b288861` | S1 CLI 核定 | lark-cli 1.0.98 真实调用：`docs +fetch`(wiki URL 自动解包)、`docs +search`、`drive +inspect`、`wiki +node-get`；scope：inspect=`drive:drive.metadata:readonly`、node-get=`wiki:node:retrieve`（均实测 missing_scope），fetch/search 现有 token 可用 | §13 已回写真实命令面与响应结构 | adapter typed 命令 + service 方法 |
| 2026-10-27 | `7b288861`+工作区 | S1/S2/S3 实现 | `cargo test -p nomifun-office-hub`（72 过/4 忽略环境用例）；`cargo test -p nomifun-knowledge --lib`（365 过）；`cargo check -p nomifun-app`；`bun test --cwd ui src/renderer/pages/knowledge/`（158 过）；`bun run typecheck`；`check:i18n`；`check:desktop-ui-boundary`；`check:agent-session-boundary`（登记 020 迁移+内容规则后通过） | 全栈接通：port 注入（`router/office_hub.rs` `OfficeFeishuKnowledgePort` + `state.rs` 组合根）、`derive_kind="feishu"`、`/api/knowledge/bases/{id}/content` `feishu` 变体、CreateStudio/详情页/工作台 UI、迁移 020 + boundary 注册、UI/API 合同 51 | S4：validation 文档 + 真实租户验收（待验） |
| 2026-10-27 | `e67f44134` | S4 收尾 | `docs/validation/feishu-knowledge-base.zh.md` 已建；office handoff §3 合同修订已登记；`check:docs` 通过；`cargo test -p nomifun-app --lib knowledge`（58 过）。注：nomifun-knowledge 集成测试中有 1 个既有 real-http 检索用例失败，与本次改动无关（未触碰该路径）。真实飞书租户端到端验收仍待执行（需已连接连接器） | 提交 `e67f44134`（main，未推送） | 真实租户验收：建库→加文档→挂载→search/read |

## 13. 已核定命令面（lark-cli 1.0.98，真实调用验证于 2026-10-27）

环境：共享 user-tool prefix `…/nomifun-user-tools/npm/lark-cli`，版本 1.0.98；profile `nomifun-office-2` 有有效 user token（真实验证），`nomifun-office` 缺 user identity。

| 知识操作 | 命令 | 底层 API | 必需 scope | 响应结构（实测） |
| --- | --- | --- | --- | --- |
| 正文读取 | `docs +fetch --doc <url|token> --doc-format markdown --as user` | POST `/docs_ai/v1/documents/{tok}/fetch` | 现有 token 已可用（实测通过） | `data.document.{content(markdown), document_id, revision_id}` |
| 元数据/索引刷新 | `drive +inspect --url <url> [--type t] --as user` | POST `/drive/v1/metas/batch_query` | `drive:drive.metadata:readonly`（实测 missing_scope 报错） | `data.metas[]`（metas/batch_query 标准形态，实测未授权未取到） |
| wiki 解析 | `wiki +node-get --node-token <tok|url> --as user` | GET `/wiki/v2/spaces/node_by_token?token=` | `wiki:node:retrieve`（实测 missing_scope 报错） | node 详情（obj_token/obj_type/space_id） |
| 远端检索 | `docs +search --query <q> --as user [--page-size n≤20] [--page-token t] [--filter json]` | POST `/search/v2/doc_wiki/search`（Search v2） | 现有 token 已可用（实测通过） | `data.{results[], has_more, page_token}`；item=`{entity_type:DOC|WIKI, result_meta:{token,doc_types,url,update_time,update_time_iso,owner_name,icon_info}, title_highlighted, summary_highlighted}` |

关键事实：

- `docs +fetch --doc` 直接接受 wiki URL/node token 并自动解包（实测：wiki URL → `document_id` 返回底层 obj_token），无需显式 wiki 解析步骤；wiki → obj_token 仅在需要 wiki 空间元数据时用 `wiki +node-get`。
- `docs +fetch --doc-format markdown` 返回纯 markdown 正文（xml 为默认，知识链路固定用 markdown）；`--detail full` 含编辑元数据但正文读取不需要。
- `drive +inspect` 用 `--url`（不是 `--doc`），自动解包 wiki、返回 type/title/canonical token——正是索引刷新的 meta-only 命令，存在且已核定。
- `docs +search` 与 `drive +search` 同走 Search v2 `doc_wiki/search`；`docs +search` 结果含 `entity_type`(DOC/WIKI) 与 `result_meta.doc_types`(DOCX/DOC/…)；wiki 命中时 `result_meta.token` 是 node_token，底层 obj_token 在 `result_meta.icon_info`(JSON 串) 的 `token` 字段。
- Search v2 分页 `page_size` 上限 20，`page_token` 为不透明串。
- 新 scope：`drive:drive.metadata:readonly`（inspect）、`wiki:node:retrieve`（node-get）。`documents.read` capability 对应 docs +fetch 所需读取 scope（既有登录已含）；inspect/node-get scope 走按需授权（missing_scope 错误已带 `missing_scopes` 列表与 device-flow hint，既有 `output_error` 分类可直接映射）。
- 限流：知识链路自建 CLI 并发信号量 ≤4；inspect 本身即 batch_query 形态，索引刷新按策展集逐条 inspect（一期 ≤64 条）。
