# 飞书办公：文档列表与页面收敛验收

对应[现役实施计划](../handoffs/office-feishu-workbench.zh.md)。文档检索验证：2026-10-26；页面收敛与连接流程重设计：2026-10-27；日程与任务整体移除见末节。

> 入口更新（2026-10）：页面现为「知识库 → 飞书知识库」（`/knowledge/feishu`，`/office`
> 重定向至此），连接设置现为「MCP & 技能 → 连接器 → 飞书连接器」。验收记录中的
> 「办公页面 / `/office`」对应这两个入口，页面合同与后端边界不变。

## 根因

1. 默认列表使用 `drive files list` 的空 `folder_token`，只能列根目录直接子项；这与用户的可访问文档集合不同，尤其遗漏知识库。
2. 列表额外要求 `space:document:retrieve`，而已连接账号原本具备的是 `search:docs:read`；用户有文档却无法进入旧列表。
3. 列表与搜索有两个 DTO、权限、CLI 命令和解析分支；默认列表没有分页，类型筛选仅作用于当前页，可能把全局存在的类型显示为空。
4. 分页令牌复用了最多 512 字符的资源 ID 校验。真实账号返回的合法游标为 537 字节，原搜索翻页也会被拒绝。
5. 读取失败与空状态同时渲染；测试主要依赖“根目录已有文档”的理想夹具，未覆盖真实知识库结构与长令牌。

## 最终架构

`Documents(query, filter, cursor)` → 精确权限检查 → 官方 `drive +search` → Search v2 → 严格元数据投影 → 列表。

- 空关键词浏览、关键词搜索、类型筛选和分页共用一条查询；权限统一为 `search:docs:read`。
- 类型通过 `--doc-types` 在服务端筛选，按 `edit_time` 排序；不增加 owner、时间、文件夹或知识空间限制。
- 删除旧根目录命令／解析器、独立搜索合同、重复权限、当前页类型筛选和 URL 充当资源 ID 的路径。
- token 来自 `result_meta.token`；知识库 DOCX 保持 `wiki` 资源身份，不开放误删。
- 原样传递不透明游标，校验与资源 ID 分离；拒绝坏分页，不用不可靠 total 宣称全部资源数。
- UI 明确区分加载、权限、错误和无匹配；查询条件或连接变化后不复用旧分页。
- 标题创建、删除回执、浏览器打开与正文不进入 NomiFun 的边界保持原合同。
- 当前 UI/API 合同为 50。正文和凭据不进入办公产品存储；重复入口对应结构由 018、019 前向迁移删除，Agent Session 合同不变。

## 真实账号只读证据

读取当前开发数据集的办公连接定位，不读取凭据文件、不登录或申请新权限。
现有 CLI 版本为 1.0.98，用户身份为 ready，已具备 `search:docs:read`。

| 路径 | 实际结果 |
| --- | --- |
| 旧根目录命令 | exit 3；`authorization / missing_scope`；缺少 `space:document:retrieve` |
| 官方 CLI 空关键词浏览第一页 | 20 项，`has_more=true`，下一页令牌 537 字节 |
| 同查询第二页 | 5 项，`has_more=false`，两页 token 无重复 |
| NomiFun 现有受监督运行器＋新适配器的显式只读探针 | 20＋5 项，投影为 23 个 wiki 和 2 个 Base |
| 服务端云文档类型筛选 | 第一页 20 项，全部 DOCX，仍有下一页 |
| 服务端 Base 类型筛选 | 2 项，全部 BITABLE，无下一页 |

探针仅读取状态和最多两页元数据；使用独立临时工作目录，不写真实云文档、不修改连接配置。
原始标题、链接、用户 ID、凭据和游标未写入验收记录。

可重复执行的探针为 `adapter::tests::live_document_discovery_read_only`，默认 ignored。
显式设置 `NOMIFUN_OFFICE_PROBE_CLI`、`NOMIFUN_OFFICE_PROBE_PROFILE` 后，
运行 `cargo test --locked -p nomifun-office-hub --lib adapter::tests::live_document_discovery_read_only -- --ignored --nocapture`。
该探针要求账号中已有文档；不会替用户连接账号或开通权限。

## 文档检索自动验证（2026-10-26）

| 检查 | 最终结果 |
| --- | --- |
| `cargo test --locked -p nomifun-office-hub --lib -- --test-threads=2` | 62 passed，4 ignored；20.54 秒 |
| 显式 `live_document_discovery_read_only -- --ignored --nocapture` | 1 passed；真实账号 25 项；2.20 秒 |
| `bun test --cwd ui src/renderer/pages/officeHub` | 移除独立 Base 页面后 46 passed，270 assertions |
| `bun run typecheck` | 通过 |
| `bun run check:desktop-ui-boundary` | 通过，最小支持视口 880×600 |
| `bun run check:i18n` | 通过 |
| `bun run check:docs` | 通过 |
| `bun run build:ui` | 移除独立 Base 页面后通过，25.15 秒，manifest 为 UI/API 49 |
| `git diff --check` | 通过 |

默认 ignored 包含三个原有子进程夹具入口和一个需显式账号的只读探针；后者已单独执行，不能把默认跳过计作真实账号证据。
自动用例覆盖真实 Search v2 元数据结构、Wiki 资源身份、原样长游标、坏分页拒绝、服务端筛选，
以及默认浏览翻页、查询／筛选／账号变更后的分页重置、游标循环与错误不冒充空结果。
既有创建／删除单次回执、pending 围栏、快照和 Agent 清理保留用例继续通过。

## 页面入口收敛

按用户 2026-10-27 的要求，顶部仅保留文档、日程与任务两个页签，默认进入文档。
多维表格继续在文档中通过类型筛选、搜索和浏览器打开；删除独立 `BaseView`、编辑表单、
页面状态、字段转换辅助函数、专用样式和文案，以及已退役页面的交互测试。
权限管理不提供独立的表内数据读取权限选项。保存结果和资源上的原操作核验继续使用单次回执。
现有 Base 列表交互测试确认顶部没有重复页签，且筛选仍提交 `filter=base` 并打开原浏览器链接。

历史列表的页面、状态、分页、前后端查询、`OperationPage` 与专用中英文文案已删除。
前向迁移 `018_remove_office_activity.sql` 删除历史排序索引及操作创建／更新时间字段；
保存所需的请求身份、逐项结果和执行回执继续保留。UI/API 合同为 50。
升级验证使用临时数据库，对照 017 已提交数据、原迁移 checksum、Agent 内容、元数据和自增高水位。

2026-10-27 两个页面清理集成后重新执行的验证：

| 检查 | 结果 |
| --- | --- |
| 办公 UI 定向测试 | 45 passed，268 assertions；默认文档、两个页签、只读文档查询和资源上的原操作核验通过 |
| 数据库合同、办公升级与压缩测试 | 14 passed，覆盖现有 017 数据库升级、专用字段及最近资源缓存删除 |
| Agent 数据清理定向测试 | 6 passed |
| Agent schema 合同测试 | 14 passed |
| 办公后端单元测试 | 62 passed，4 ignored；已移除的历史列表 GET 返回 405，最近资源 POST 返回 404，单次保存和核验继续可用 |
| 类型、i18n、文档、desktop UI 与 Agent Session 边界 | 通过；边界脚本 3 项测试通过 |

本次未重跑真实账号探针或桌面浏览器验收，未修改实际使用的数据集。

## 工作台概览移除

重复的概览页、最近打开列表、右侧当日日程与任务摘要及专用样式已删除。
文档页只读取文档；日程与任务在对应页签读取，不再为概览读取摘要。
最近资源查询、资源打开后的持久化接口、`ResourceLink`、`OfficeItem.opened_at`
及相关中英文文案退出当前合同。

`019_remove_office_overview.sql` 删除连接表中的 `recent_resources_json`；
连接配置、单次请求身份、未解决目标和执行回执继续保留。
已发布迁移保持原文与 checksum，移除页面不授予 Agent 数据重建或导入权限。
本轮定向测试、类型与边界检查结果见上表；数据库升级仅在临时数据中执行。

## 连接体验与账号管理重设计

首次连接按实际状态显示下一步，不再把组件、账号和默认勾选权限串成必经向导：

1. 未安装组件时，主页面提供明确的安装说明和入口，以及安装进度、失败重试状态。
2. 组件就绪后选择或配置应用，再打开浏览器完成用户登录；基础登录只请求 `offline_access`。
3. 已登录后从页头当前账号进入管理，默认展示账号信息；权限按使用申请，组件诊断和停用归入高级设置。
4. 更换账号在确认并退出后重新登录；退出登录只退出；切换连接选择另一个配置；停用不冒充退出。

新增 `POST /api/office-hub/auth/logout`，请求包含 `profile` 与可空的 `expected_actor`。
实际 profile 和用户身份必须匹配，退出前后使用官方用户清单核验。
活动写入、持久化 queued/running 操作、未收尾授权和正在准备组件都拒绝退出。
HTTP 调用方离开不撤销原 Host owner 对退出与核验的责任。

官方 [CLI 1.0.98 logout](https://github.com/larksuite/cli/blob/v1.0.98/cmd/auth/logout.go)
清除所选 profile 的全部用户，不支持精确单用户退出。
因此仅零或单用户 profile 可自动退出；多用户配置会明确拒绝并提示“切换连接”，不会连带注销其他账号。
此范围通过 [auth list](https://github.com/larksuite/cli/blob/v1.0.98/cmd/auth/list.go) 的正式 JSON 核验，
不读取或复制凭据文件，不删除应用连接或已提交的业务回执。

本轮已执行办公后端定向测试：67 passed、4 ignored、0 failed。
新增用例覆盖精确 CLI argv 和回执形态、单用户退出后重新登录、过期／空身份、重复退出、
身份与连接变化、多用户拒绝、无活 worker 的持久化排队操作，以及活动执行和授权收尾围栏。
测试只使用临时数据库与确定性执行器，没有执行任何真实账号登录、退出、授权或 token 撤销。
本轮办公 UI 定向测试重新执行：54 passed、338 assertions、0 failed。
类型、i18n、文档、desktop UI 与 Agent Session 边界检查通过；UI 生产构建通过，合同为 UI/API 50。
覆盖未安装与失败重试、组件损坏、登录过期、停用状态只读设置、配置后再登录、
已连接默认账号管理、退出不重登、换号等待原退出、退出失败不发新登录，以及部分授权继续按需补充。
原文档分页、精确删除、原请求回执、身份变化及业务补权保留输入的用例继续通过。

实际 renderer 的离线浏览器验收在 1280×720 和 880×600 桌面视口执行：

- 未安装页面可显式准备，先显示准备状态，完成后显示登录入口。
- 模拟登录后直接显示当前账号；退出确认后回到未登录，未自动发起新登录。
- 更换账号确认后观察新登录流程，模拟另一个账号完成后，页头与账号面板显示新身份。
- 权限独立显示实际未授权状态；账号管理、访问权限和高级设置在最小窗口下可操作。
- 登录过期显示重新登录入口；组件损坏显示检查入口，不提供重复安装。

上述浏览器操作均通过离线夹具模拟；没有访问真实凭据、授权页面或云文档。

## 浏览器范围与限制

[离线验收页](../../ui/test/office-hub-browser.README.md)导入实际 renderer，
业务与浏览器打开均为模拟；补充 25 项无关键词翻页、全局类型筛选、查询错误和空查询场景。
2026-10-26 已在 Codex 内置桌面浏览器的 1280×720 视口执行：

- 不输入关键词时显示第一页 20 项；点击下一页后显示 21～25，上一页可用、下一页不可用。
- 查询错误场景显示错误和“重新检查”，不同时显示空文档断言。

真实 CLI 读取与浏览器模拟交互分别计为证据，不互相替代。
连接重设计已补充 880×600 的实际 renderer 浏览器截图；文档分页的原浏览器证据日期仍为 2026-10-26。

搜索结果仍由飞书 ACL、可见性与索引决定；新建文档进入索引可能有延迟。
真实云端创建／删除本次未执行。已有桌面进程未被终止或替换，源码与 UI 构建不等于安装新发行包。

## 日程与任务整体移除

按用户要求，日程与任务的设计与代码整体退出当前合同：

- 后端 `OfficeQuery` 仅剩 `documents` 与 `base_records`；`OfficeMutation` 仅剩
  `document_create`、`document_delete`、`base_update`。任务、清单、人员检索和日历的
  查询／变更、CLI 命令构造、解析、权限映射（`tasks.*`、`tasklists.*`、`calendar.*`、
  `people.read`）及对应的准入、回执与核验分支已删除。
- 前端移除「日程与任务」页签、任务列表行组件、任务／日程表单、日期选择、
  权限分组和全部相关文案；页面直接展示文档列表，无业务页签。
- 四张办公表不保存任务／日程专属结构，无 schema 变更；历史任务操作仅作为
  不可执行的只读 JSON 回执证据保留，`task_create` 测试夹具已换成 `document_create`。
- 运行时探针曾用 `schema drive.files.search` 验证命令契约；该方法并不存在于官方
  CLI，已改为只核验可启动性与 `--version` 输出的 lark-cli 身份，无版本门。

本轮执行的验证：

| 检查 | 结果 |
| --- | --- |
| `cargo test --locked -p nomifun-office-hub --lib -- --test-threads=2` | 66 passed，4 ignored |
| `cargo test --locked -p nomifun-db --test office_receipts_compact` | 6 passed |
| `bun test --cwd ui src/renderer/pages/officeHub` | 52 passed，326 assertions |
| `bun run typecheck` | 通过 |
| `bun run check:i18n` | 通过，i18n-keys 已重新生成 |
| `bun run check:desktop-ui-boundary` | 通过，最小支持视口 880×600 |
| `bun run check:docs` | 通过 |

本轮未执行真实飞书账号操作与桌面浏览器验收；此前按日期记录的验收条目
描述当时的页面结构，不代表当前合同。

## 官方基线

- [CLI 1.0.98 统一检索实现](https://github.com/larksuite/cli/blob/v1.0.98/shortcuts/drive/drive_search.go)
- [空关键词、过滤与分页合同](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-drive/references/lark-drive-search.md)
- [根目录与文件夹直接子项语义](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-drive/references/lark-drive-files-list.md)
