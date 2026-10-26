# 办公：飞书实施计划

状态：按用户 2026-10-27 的要求收敛页面并重做连接、账号与权限流程，当前验证以[验收记录](../validation/office-feishu-workbench.zh.md)为准。
本文是本功能唯一现役计划。真实飞书租户与各发行平台的验证分别记录，未执行不计为通过。
所有业务读取和写入由官方飞书 CLI 完成。

## 1. 产品范围

一级入口为「服务 → 办公」，页面为 `/office`，首期只开放飞书。
支持 Tauri 桌面和桌面 WebUI，最小视口 880×600，使用共享 PageHeader、主题和双语。

- 文档：列表、搜索、标题创建、删除与浏览器链接打开。NomiFun 不读取、预览或编辑文档正文。
- 日程与任务：个人日程、我负责／我创建／指定清单的任务，创建、完成和重新打开。
- 连接状态：主页面明确处理组件缺失、应用未连接、未登录与登录过期；已连接时从页头账号入口管理账号。

页面只保留文档、日程与任务两个页签，默认进入文档。

不扩展完整飞书客户端、其他平台驱动、多人账号池或协同富文本编辑器。
本地 Office 文件预览、渠道机器人和 Agent Session 的身份与事实链保持原合同。

## 2. 连接状态与账号管理

### 首次连接：根据当前状态继续

组件缺失时，主内容显示安装说明和明确的安装入口，安装期间显示进度、完成或重试结果。
优先复用已有兼容 CLI；只有用户明确安装时才允许供应官方 npm 包 `@larksuite/cli@1.0.98`。
启动应用、进入页面、读取状态和探测已有程序都不下载 Node 或 CLI，CLI 不进入 NomiFun 安装包。
已有安装损坏或不兼容时展示修复入口，不将其误判为缺失并另行安装。

组件就绪后选择已有应用连接，或通过官方流程配置新应用，再打开浏览器完成用户登录。
显示真实应用和用户信息，不用样例企业名或 profile 名推断租户身份。
每次 CLI 调用显式使用 profile，不改全局默认，不把一组应用凭据当作并行多人账号池。
基础登录只申请官方 `offline_access`，不以空 scope 或裸登录触发全域业务授权。
登录完成即进入业务页面，不再把权限选择作为必经的最后一步。

### 已连接：账号是默认管理入口

页头显示当前用户；打开后默认展示当前账号、应用连接及可执行的账号操作。
“更换账号”在确认后退出当前连接的登录状态，再进入用户登录；“退出登录”只退出，不自动发起登录。
“切换连接”选择另一个应用配置。已登录用户不用重复经过安装或权限设置才能查看这些操作。
登录过期回到账号登录提示，连接未启用则明确提供启用入口。

退出调用 `POST /auth/logout`，请求包含确切的 `profile` 和 `expected_actor`（可为空以处理已过期身份）。
后端在现有准入围栏内检查连接、预期身份、活动操作、持久化 queued/running 项及未收尾授权，
使用所选 profile 的官方 `auth list --json` 证明其为零或单用户，再调用 `auth logout --json` 并复核空列表。
HTTP 观察者离开不会取消原退出 owner，也不使另一次登录越过尚未完成的退出。
该过程不删除应用配置、已提交请求和未知结果栅栏，不把凭据复制到 NomiFun。

官方 CLI 1.0.98 的 [logout](https://github.com/larksuite/cli/blob/v1.0.98/cmd/auth/logout.go)
会清除所选 profile 中的全部用户，不能精确注销其中一个用户。
因此多用户 profile 拒绝自动退出并提示使用“切换连接”，不注销其他账号或其他 profile。
用户清单只通过官方 [auth list](https://github.com/larksuite/cli/blob/v1.0.98/cmd/auth/list.go) 临时核验。

### 权限按使用申请，组件与停用放入高级设置

权限区显示文档、日程与任务所需的读取能力及真实授权状态，不默认勾选一组权限。
首次使用缺失能力时请求该范围；用户确认、管理员等待、拒绝和过期分别展示。
读取权限不代表写权限；资源成员权限也不能由 OAuth scope 推导。
实际保存缺少写权限时，围绕当前操作请求必要权限。授权结束回到原输入，仍需用户点击保存。
若 app、用户或 profile 变化，旧表单停止提交，不把旧输入自动绑定到新身份。

组件主机、版本、指定绝对路径、恢复自动发现、只读更新检查和停用连接归入高级设置。
停用停止新业务准入，保留连接及已提交回执；停用不是退出登录，也不是撤销飞书授权。
官方完整向导仅供用户在自己的终端执行，可能安装上游全局 Skills，不由办公页静默启动。
没有多个安装模式、项目内安装或由 Skill 启动的第二个共享安装器。

配置、授权和取消都有原 Host owner。前端观察同一个 flow，响应丢失时接回原流程。
取消先进入收尾状态，原进程树确认清理后才结束；重复点击不产生另一条授权。
密钥与 token 仅留在官方 CLI 存储中，device code 只留在本次 Host 内存。

## 3. 简单、直接的业务交互

### 文档

文档列表、关键词搜索和类型筛选共用一个 `Documents` 查询合同：`query`、`filter`、`cursor`。
统一通过官方 CLI `drive +search` 调用 Search v2；空关键词直接浏览当前账号可检索的文档，
不加 owner、时间、根目录或知识空间限制。按更新时间排序，每页 20 项，保留服务端分页。
`all`／`document`／`base` 映射为 CLI `--doc-types`，筛选先于服务端分页，不在当前页冒充全局筛选。
资源范围仍受飞书 ACL、应用可见性和搜索索引约束，不承诺租户所有资源或即时索引。

列表与搜索只申请 `search:docs:read`，不再为默认列表申请独立根目录权限。
删除 `drive files list` 根目录入口、独立 `Search` DTO 和两套能力／解析分支；不保留回退。
返回项只投影 `result_meta.token`、类型、标题、浏览器链接和更新时间，丢弃正文摘要与不可靠 total。
知识库资源保持 `wiki` 身份，即使内部文档类型是 DOCX，也不因此获得 Drive 文档删除入口。

分页令牌属于服务端不透明数据，不使用资源 ID 的 512 字符校验；保留原值并使用独立的 16 KiB 传输预算。
拒绝缺失或重复的有效下一页令牌；无下一页时忽略上游残留 token。
前端把分页状态绑定到连接、权限、查询和筛选，改变条件后回到第一页，拒绝重复游标循环。
读取失败、缺少权限、正在读取和查询无结果分别显示；失败不显示“账号没有文档”。

点击文档名或「浏览器打开」直接交给系统浏览器。
多维表格统一在文档列表中搜索、筛选和通过浏览器打开；没有独立页签或应用内字段／记录编辑表单。
链接必须是飞书／Lark 官方 HTTPS 资源地址，知识库链接不解析成另一个可删除对象。
没有可用链接时明确提示通过搜索或粘贴完整链接，不拼接未知租户地址。

新建只填写标题，通过 CLI 的标题专用能力创建空白文档；创建后回到列表，正文在浏览器中编写。
删除先显示具体标题和链接，用户点击删除后一次提交封闭请求。
删除只对确切 token 与已支持类型的云文档开放，不删除文件夹、快捷方式、知识库节点或多维表格。
使用官方 CLI `drive +delete --file-token ... --type ... --yes`，保留身份、幂等请求和原始回执。
只有相同 token／type 的 `deleted=true` 终态回执才能确认成功；异步待完成、超时和不完整回执继续待核验，不能重发删除。

读取权限只申请列表元数据和搜索，创建／删除权限按需补充；不再申请正文读取或段落写入权限。
移除文档详情页面、Markdown 导出阅读、原生 block 读取／分页／编辑、正文创建输入与对应 DTO、样式和测试。
UI/API 合同为 50；根目录查询和独立搜索合同、文档正文查询／写入均不可执行。
已有操作的原始 JSON 回执保持只读证据，未解决的原文档栅栏继续阻止同一资源的新删除。
文档正文不落库，Agent Session 合同保持不变，不恢复旧执行合同。

### 日程与任务

个人日程明确时间与时区，不隐式添加参与者或视频会议。
任务手工填写标题、真实人员 ID 和可选截止日期；创建人、负责人、目标清单分别显示。
「我负责」和「我创建」用不同 CLI 查询；指定清单必须具备相应访问权。
完成和重新打开任务同样需要写权限。

## 4. 单次保存与结果合同

表单本身就是用户检查输入的位置。点击保存后只提交一次封闭业务请求，
不增加持久化的中间确认阶段，也不接收前端传来的 CLI argv、Shell 或任意 API 路径。

1. 请求携带稳定 `request_id`、预期 app／用户／profile 和确切输入。
2. 后端检查身份、权限与当前资源条件，在同一事务内保存已提交操作和目标唯一栅栏。
3. 每项调用 CLI 前先提交精确的 running 尝试记录；HTTP 连接不拥有业务 worker。
4. 响应丢失后按原 request ID 查询，不以页面刷新、取消观察或超时为由再次写入。
5. 原尝试只结算一次，最终回执不可改写。后续只读核验另增观察，保留原超时与原输入。
6. 重启后已开始但缺最终证明的项成为 unknown；未跨启动栅栏的项才明确为未执行。
7. 成功、明确未执行、未知和已接受待确认逐项展示，批量操作允许部分完成。

只有有未执行证明的项目才能修正后显式提交新请求，旧请求与旧回执保留。
创建 nonce 跨 profile 和用户切换仍不能重复使用已成功或未解决的创建；新表单使用新 nonce。
未取得任务 GUID 或确切创建回执时，不凭同名搜索宣布成功。
待核验的已知资源使用跨身份目标栅栏；同一文档的未解决操作继续阻止新的删除。

停用停止新业务准入并撤回尚未开始的项目，保留配置和已提交请求回执。
已开始的 CLI 仍由原 owner 收尾，停用不等于撤销飞书服务端授权。
保存结果在当前表单中提示；资源上的未解决操作入口继续读取原结果并核验。

## 5. 最小持久化

使用主库现役产品表规则：整数自增本地主键、命名 UUIDv7 业务身份、已登记逻辑关系。
最终只保留以下四张办公表，均为非 Agent 数据并在 Agent 数据清理中 Preserve。

| 表 | 必须保留的内容 |
| --- | --- |
| `office_connections` | 启用、profile 与 CLI 路径配置 |
| `office_operations` | 已提交请求、内部幂等摘要、冻结身份与汇总状态 |
| `office_operation_items` | 精确输入、目标唯一栅栏和逐项结果 |
| `office_operation_receipts` | attempt 与 observation 两类业务记录 |

attempt 在启动前保存，running 只允许一次最终结算；observation 只追加。
对外的尝试与观察列表来自同一回执表，不另建回放引擎、投影账本或 Agent Effect。
历史未知输入保留为只读 JSON 证据，不注册旧的可执行操作类型。

文档列表／搜索读取当前资源元数据，正文仅在浏览器打开。
未提交表单不落库，已提交输入只为精确执行、幂等和未知结果核验保留。

`016_office_hub.sql` 保持原文与 checksum。新增 `017_compact_office_receipts.sql` 做精确前向收敛：
保留已提交操作、原业务身份、回执文本、未解决目标和自增高水位；
只有确认没有执行事实的未提交记录可以删除。坏形态、未知元数据或冲突整笔回滚。
Agent 的 generation、基线和 canonical 事实不变，不获得破坏性重建权限。
主库快照保留这四表；备份恢复继续遵守现役 lineage 校验，不改写备份源来取得兼容性。

`018_remove_office_activity.sql` 删除历史列表专用的 `idx_office_operations_owner_history`
索引及 `office_operations.created_at`、`updated_at` 字段。现有四表继续承载保存、幂等、
未解决目标和原回执核验；不提供操作历史列表、分页或列表查询 API。
已发布迁移 checksum、业务身份、执行回执及 canonical Agent 事实和元数据保持原值。

`019_remove_office_overview.sql` 删除 `office_connections.recent_resources_json`。
概览页面、最近资源读写、打开时间字段和侧栏摘要一并退出当前合同；
四张办公表继续承载连接、单次提交和回执核验，不保存最近打开列表。

## 6. 供应与代码边界

| 现有能力 | 复用合同 |
| --- | --- |
| `user_cli_root` | OS 用户共享的持久 npm prefix，不按工作区或 Session 重装 |
| `prepare_existing_process_launch` | 绝对现存文件的纯环境准备，不供应运行时、不创建目录 |
| `prepare_process_launch` | 仅在显式准备组件时允许相对 npm 命令供应缺失运行时 |
| `nomi-process-runtime` | 字面 argv、监督启动、有限输出、取消和原进程树清理证明 |
| `protect_instance_owner` | 所有办公 API 只允许实例 owner，CLI 在后端 OS 账号执行 |
| `httpRequest` | 既有鉴权、CSRF、请求 ID、取消观察和结构化错误 |
| 内置 `lark-cli` Skill | 共享发现；缺失转办公连接，已有业务继续使用 canonical process |

发现顺序为选定绝对路径、共享 prefix、PATH、已有 Node/npm 的原全局 prefix。
官方 npm wrapper 只解析到已存在原生程序；缺少 native 时报告损坏，不通过诊断触发下载。
Host 对共享 prefix 的安装有跨进程锁；文档与 Skill 指引不是外部 npm 操作的锁。
当前兼容基线为 1.0.98，只读版本检查区分已安装、最新和已验证版本；不自动升级。
办公页路径与 profile 选择不会自动注入 Agent，Skill 每个 Session 重新发现并显式选身份。

`nomifun-office-hub` 持有云办公适配、服务与回执；`nomifun-office` 继续持有本地文件预览。
CLI 凭据只由官方 store 保管，不向前端返回秘密，不让 Agent 获取 trust token 调用宿主 API。

## 7. HTTP 合同

API 前缀为 `/api/office-hub`，沿用 ApiResponse 与结构化业务错误。

| API | 用途 |
| --- | --- |
| `GET /status` | 只读状态与已有组件探测 |
| `GET /profiles` | 账号连接区显式发现已有 CLI 配置，不登录或启用连接 |
| `POST /runtime/prepare` | 显式开始或加入组件准备 |
| `POST /runtime/select` | 选择现存路径或恢复自动发现 |
| `POST /runtime/check-update` | 只读上游版本检查 |
| `PUT /connection` | 启停与 profile 选择 |
| `POST /auth`、`GET /auth/{id}`、`DELETE /auth/{id}` | 开始、观察与取消原流程 |
| `POST /auth/logout` | 按 profile、expected_actor 核验并退出当前单用户连接，保留应用配置 |
| `POST /query` | 封闭的文档列表／搜索、人员、任务、日历与 Base 查询 |
| `POST /operations` | 一次提交 ExecuteRequest |
| `GET /operations/request/{request_id}` | 恢复原提交结果，不重发 |
| `GET /operations/{id}` | 读取原提交的逐项结果 |
| `POST /operations/{id}/reconcile` | 只读核验原目标 |

## 8. 验证与交付

按组件、连接、只读查询、单次保存和持久化收敛逐项集成。必要验证包括：

- 冷启动与状态读取无安装副作用；已有工具复用、坏安装、共享准备并发与取消。
- 缺失组件安装、应用连接与用户登录状态，账号退出／更换、切换连接、权限按需和高级诊断。
- 退出前后的精确 profile／身份、零或单用户条件、多用户拒绝，活动写入与授权收尾围栏。
- 管理员等待、拒绝、过期和精确收尾；停用与退出不互相冒充。
- 已有知识库／云空间文档使用统一浏览查询；空关键词分页、服务端类型筛选、长游标和错误状态均有覆盖。
- 浏览器打开保留原链接；标题创建与删除确认不出现正文预览／编辑。
- 默认文档入口、文档与日程任务两个页签，文档浏览不触发日程或任务摘要读取。
- 响应丢失按 request 查询；身份变化、重复创建、删除与 Base pending 围栏不能绕过。
- 部分完成、unknown 不重发、未执行的新请求、原回执不可改写与读回不假成功。
- 016→017 历史输入／回执、序列高水位、坏形态原子拒绝、快照和 Agent 清理保留。
- 018 删除历史列表专用结构，019 删除最近打开配置，连接与单次请求回执保持原值。
- 880×600 与桌面宽窗、PageHeader、双语、主题、键盘弹窗和真实错误／空状态。

运行目标 Rust、DB/schema/备份测试、前端交互与类型检查，以及 desktop UI 与 Agent Session 边界检查。
实际执行结果、平台限制和真实飞书端未执行范围统一记录到[验收记录](../validation/office-feishu-workbench.zh.md)。

## 9. 官方契约基线

- [官方 CLI 1.0.98](https://github.com/larksuite/cli/tree/v1.0.98)
- [完整安装向导](https://github.com/larksuite/cli/blob/v1.0.98/scripts/install-wizard.js)
- [退出登录及 profile 范围](https://github.com/larksuite/cli/blob/v1.0.98/cmd/auth/logout.go)
- [已登录用户清单](https://github.com/larksuite/cli/blob/v1.0.98/cmd/auth/list.go)
- [登录与显式权限参数](https://github.com/larksuite/cli/blob/v1.0.98/cmd/auth/login.go)
- [文档创建](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-doc/references/lark-doc-create.md)
- [统一资源检索](https://github.com/larksuite/cli/blob/v1.0.98/shortcuts/drive/drive_search.go)
- [列表浏览与筛选合同](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-drive/references/lark-drive-search.md)
- [Drive 删除](https://github.com/larksuite/cli/blob/v1.0.98/shortcuts/drive/drive_delete.go)
- [任务创建](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-task/references/lark-task-create.md)
- [个人日程行为](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-calendar/references/lark-calendar-create.md)
- [Base 批次回执](https://github.com/larksuite/cli/blob/v1.0.98/shortcuts/base/record_batch_update.go)
