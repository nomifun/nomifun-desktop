# 办公工作台：飞书实施计划

状态：按用户确认的 compact-v4 产品合同实施收敛，当前验证以[验收记录](../validation/office-feishu-workbench.zh.md)为准。
本文是本功能唯一现役计划。真实飞书租户与各发行平台的验证分别记录，未执行不计为通过。
所有业务读取和写入由官方飞书 CLI 完成。

## 1. 产品范围

一级入口为「常用 → 办公」，页面为 `/office`，首期只开放飞书。
支持 Tauri 桌面和桌面 WebUI，最小视口 880×600，使用共享 PageHeader、主题和双语。

- 概览：真实连接状态、已打开资源入口、当日日程和我负责的任务。
- 文档：搜索、打开、新建，以及文档内的普通段落编辑。
- 日程与任务：个人日程、我负责／我创建／指定清单的任务，创建、完成和重新打开。
- 多维表格：链接定位、真实字段和记录分页、简单可写字段的批量修改。
- 操作记录：当前工作台提交的结果、部分完成和待核验项。
- 连接弹窗：三步完成组件、账号与读取权限设置；写权限按需补充。

不扩展完整飞书客户端、其他平台驱动、多人账号池或协同富文本编辑器。
本地 Office 文件预览、渠道机器人和 Agent Session 的身份与事实链保持原合同。

## 2. 三步连接弹窗

### 第一步：准备环境

展示后端运行主机和组件状态，优先复用已有兼容 CLI。只有用户明确准备时才允许安装；
启动应用、进入页面、读取状态和探测已有程序都不下载 Node 或 CLI。
缺失时由唯一 Host 供应器准备官方 npm 包 `@larksuite/cli@1.0.98`，不进入 NomiFun 安装包。

高级项保留指定绝对路径、恢复自动发现、只读版本检查，以及官方完整向导命令。
完整向导仅供用户在自己的终端执行，可能安装上游全局 Skills，不由工作台静默启动。
没有多个安装模式、项目内安装或由 Skill 启动的第二个共享安装器。

### 第二步：连接飞书

默认走官方企业应用配置与用户登录链接。复用现有 profile 或手工企业应用信息置于高级项。
显示真实应用和用户信息，不用样例企业名或 profile 名推断租户身份。
每次 CLI 调用显式使用 profile，不改全局默认，不把一组应用凭据当作并行多人账号池。
基础用户登录显式申请官方 `offline_access`，不以空 scope 或裸登录触发全域业务授权。
第三步再选择业务读取权限；稍后授权时不把尚未授予的能力显示为可用。

配置、授权和取消都有原 Host owner。前端观察同一个 flow，响应丢失时接回原流程。
取消先进入收尾状态，原进程树确认清理后才结束；重复点击不产生另一条授权。
密钥与 token 仅留在官方 CLI 存储中，device code 只留在本次 Host 内存。

### 第三步：选择读取权限

按文档、日程、任务和多维表格选择需要的读取能力，可稍后补充。
显示已授予、缺失、用户确认、管理员等待、拒绝和过期等真实状态。
读取权限不代表写权限；资源成员权限也不能由 OAuth scope 推导。

实际保存缺少写权限时，围绕当前操作请求必要权限。授权结束回到原输入，仍需用户点击保存。
若 app、用户或 profile 变化，旧表单停止提交，不把旧输入自动绑定到新身份。

## 3. 简单、直接的业务交互

### 文档

新建表单包含标题和可选正文。空正文走 CLI 的标题专用创建能力。
打开已有文档时在文档内显示原生内容，不把全文放入一个转换后的 Markdown 编辑框。

编辑前必须读到真实 revision 和原生 block 身份。只允许编辑已确认可写的普通纯文本段落，
一次选择一个段落并点击保存；标题、复杂样式、表格、嵌套和不支持的 block 保持只读。
没有可靠编辑基线时提供阅读与在飞书打开，不伪造 block ID，也不把整篇内容重建成新文档。
只有后端确认正文为空时，才允许新增首段。

保存绑定原文档、原 block 和选择时的 revision。提交前重新检查基线；源内容变化时保留输入并要求重新读取。
一个文档存在 queued、running 或未解决结果时，整篇文档保持写保护，换段落不能绕过。
原生段落写入仍可能与其他飞书客户端并发，不承诺跨程序的排他编辑。

### 日程与任务

个人日程明确时间与时区，不隐式添加参与者或视频会议。
任务手工填写标题、真实人员 ID 和可选截止日期；创建人、负责人、目标清单分别显示。
「我负责」和「我创建」用不同 CLI 查询；指定清单必须具备相应访问权。
完成和重新打开任务同样需要写权限。

### 多维表格

普通入口粘贴含 `/base/` 与 `table` 参数的数据表链接，高级项保留明确坐标和筛选。
字段、可选项、记录身份与分页均来自实际 CLI 响应，内部写入键使用 field ID。
选中记录、填写简单字段值后一次保存；同一请求只针对一个数据表，最多 200 条。
系统、公式和不支持的复杂字段不开放普通值编辑。

提交前复核字段与原值。预检不等于飞书平台的原子 CAS，不能阻止其他客户端同时修改。
批次接受回执不等于每条记录成功；按原 record ID 和目标字段读回确认。
未观察到目标值继续待核验，不据此重发整批；待核验行不能再次选中写入。

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
待核验的已知资源使用跨身份目标栅栏；旧文档写入的未解决结果继续锁住整篇文档。

停用停止新业务准入并撤回尚未开始的项目，保留配置、最近入口和历史回执。
已开始的 CLI 仍由原 owner 收尾，停用不等于撤销飞书服务端授权。
操作记录仅覆盖工作台发起的操作，不冒充外部 CLI、渠道或全部 Agent 调用的审计。

## 5. 最小持久化

使用主库现役产品表规则：整数自增本地主键、命名 UUIDv7 业务身份、已登记逻辑关系。
最终只保留以下四张办公表，均为非 Agent 数据并在 Agent 数据清理中 Preserve。

| 表 | 必须保留的内容 |
| --- | --- |
| `office_connections` | 启用、profile、CLI 路径与薄导航配置 |
| `office_operations` | 已提交请求、内部幂等摘要、冻结身份、汇总状态与时间 |
| `office_operation_items` | 精确输入、目标唯一栅栏和逐项结果 |
| `office_operation_receipts` | attempt 与 observation 两类业务记录 |

attempt 在启动前保存，running 只允许一次最终结算；observation 只追加。
对外的尝试与观察列表来自同一回执表，不另建回放引擎、投影账本或 Agent Effect。
历史未知输入保留为只读 JSON 证据，不注册旧的可执行操作类型。

最近入口放入连接的 `recent_resources_json`，最多 100 条，仅保存资源定位、标题与打开时间。
不缓存正文、记录字段、权限和 pending 状态；每次打开仍从 CLI 读取当前数据。
未提交表单不落库，已提交输入只为精确执行、幂等和未知结果核验保留。

`016_office_hub.sql` 保持原文与 checksum。新增 `017_compact_office_receipts.sql` 做精确前向收敛：
保留已提交操作、原业务身份、回执文本、未解决目标和自增高水位；
只有确认没有执行事实的未提交记录可以删除。坏形态、未知元数据或冲突整笔回滚。
Agent 的 generation、基线和 canonical 事实不变，不获得破坏性重建权限。
主库快照保留这四表；备份恢复继续遵守现役 lineage 校验，不改写备份源来取得兼容性。

## 6. 供应与代码边界

| 现有能力 | 复用合同 |
| --- | --- |
| `user_cli_root` | OS 用户共享的持久 npm prefix，不按工作区或 Session 重装 |
| `prepare_existing_process_launch` | 绝对现存文件的纯环境准备，不供应运行时、不创建目录 |
| `prepare_process_launch` | 仅在显式准备组件时允许相对 npm 命令供应缺失运行时 |
| `nomi-process-runtime` | 字面 argv、监督启动、有限输出、取消和原进程树清理证明 |
| `protect_instance_owner` | 所有办公 API 只允许实例 owner，CLI 在后端 OS 账号执行 |
| `httpRequest` | 既有鉴权、CSRF、请求 ID、取消观察和结构化错误 |
| 内置 `lark-cli` Skill | 共享发现；缺失转工作台，已有业务继续使用 canonical process |

发现顺序为选定绝对路径、共享 prefix、PATH、已有 Node/npm 的原全局 prefix。
官方 npm wrapper 只解析到已存在原生程序；缺少 native 时报告损坏，不通过诊断触发下载。
Host 对共享 prefix 的安装有跨进程锁；文档与 Skill 指引不是外部 npm 操作的锁。
当前兼容基线为 1.0.98，只读版本检查区分已安装、最新和已验证版本；不自动升级。
工作台路径与 profile 选择不会自动注入 Agent，Skill 每个 Session 重新发现并显式选身份。

`nomifun-office-hub` 持有云办公适配、服务与回执；`nomifun-office` 继续持有本地文件预览。
CLI 凭据只由官方 store 保管，不向前端返回秘密，不让 Agent 获取 trust token 调用宿主 API。

## 7. HTTP 合同

API 前缀为 `/api/office-hub`，沿用 ApiResponse 与结构化业务错误。

| API | 用途 |
| --- | --- |
| `GET /status` | 只读状态与已有组件探测 |
| `GET /profiles` | 连接向导显式发现已有 CLI 配置，不登录或启用连接 |
| `POST /runtime/prepare` | 显式开始或加入组件准备 |
| `POST /runtime/select` | 选择现存路径或恢复自动发现 |
| `POST /runtime/check-update` | 只读上游版本检查 |
| `PUT /connection` | 启停与 profile 选择 |
| `POST /auth`、`GET /auth/{id}`、`DELETE /auth/{id}` | 开始、观察与取消原流程 |
| `POST /query` | 封闭的文档、人员、任务、日历与 Base 查询 |
| `POST /operations` | 一次提交 ExecuteRequest |
| `GET /operations/request/{request_id}` | 恢复原提交结果，不重发 |
| `GET /operations`、`GET /operations/{id}` | 操作记录和逐项结果 |
| `POST /operations/{id}/reconcile` | 只读核验原目标 |

## 8. 验证与交付

按组件、连接、只读查询、单次保存和持久化收敛逐项集成。必要验证包括：

- 冷启动与状态读取无安装副作用；已有工具复用、坏安装、共享准备并发与取消。
- 三步连接、权限稍后补充、管理员等待、拒绝、过期和精确收尾。
- 普通段落使用真实 block 与 revision；复杂内容只读；可靠空正文才能插入首段。
- 响应丢失按 request 查询；身份变化、重复创建、文档与 Base pending 围栏不能绕过。
- 部分完成、unknown 不重发、未执行的新请求、原回执不可改写与读回不假成功。
- 016→017 历史输入／回执、序列高水位、坏形态原子拒绝、快照和 Agent 清理保留。
- 880×600 与桌面宽窗、PageHeader、双语、主题、键盘弹窗和真实错误／空状态。

运行目标 Rust、DB/schema/备份测试、前端交互与类型检查，以及 desktop UI 与 Agent Session 边界检查。
实际执行结果、平台限制和真实飞书端未执行范围统一记录到[验收记录](../validation/office-feishu-workbench.zh.md)。

## 9. 官方契约基线

- [官方 CLI 1.0.98](https://github.com/larksuite/cli/tree/v1.0.98)
- [完整安装向导](https://github.com/larksuite/cli/blob/v1.0.98/scripts/install-wizard.js)
- [文档创建](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-doc/references/lark-doc-create.md)
- [任务创建](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-task/references/lark-task-create.md)
- [个人日程行为](https://github.com/larksuite/cli/blob/v1.0.98/skills/lark-calendar/references/lark-calendar-create.md)
- [Base 批次回执](https://github.com/larksuite/cli/blob/v1.0.98/shortcuts/base/record_batch_update.go)
