# Agent Session 历史债务盘查与整改

本次盘查确认了历史兼容分支、未接线的并行合同和过期设计材料仍在影响重构。原有退役门禁按旧路径和关键词计数，没能发现新 canonical Store 内部的投影降级读取、字段多写及无调用的第二套 checkpoint 合同；合同生成器还要求保留历史设计材料。

本轮按用户确认直接实施删除，已物理删除 222 个旧文件，其中 179 个为文档和计划材料，43 个为代码、旧迁移或夹具。目标是一个 AgentSession 身份、一条 canonical 事件与内容链、一个 native Runtime 恢复机制、一份完整数据库 schema 源，以及一个当前架构文档入口。源基线为 `2ddf08dbc`，真实用户数据库未作为施工对象。

当前规范见 [Agent Session 日志与内容架构](../architecture/agent-session.zh.md)。本报告记录可核对的发现和实施结果，不作为另一份接口或 schema 定义。

## 盘查范围与判断方法

检查从 Session 创建、准入和持久化一路覆盖到 Runtime journal、模型历史、checkpoint、Message 展示、fork、Agent 切换、渠道绑定及领域消费者。核对 schema、真实调用点、DTO、事件生产者、测试和文档生成依赖；不以 `legacy`、`historical` 或 `Conversation` 的词面出现判定债务。

| 范围 | 核对内容 |
| --- | --- |
| Contracts 和 DB | 完整与隔离 schema、数据代际、字段、事件 registry、迁移、reset、恢复接口 |
| Runtime 和 Session owner | 结构化 journal、模型角色、内容序列、tool receipt、准入失败、切换、fork |
| UI | message-history、stream、错误、工具分组、产物、创建草稿、暂停和只读状态 |
| 领域消费者 | Channel、Cron、Companion、IDMM、AgentExecution、Requirements、AutoWork、Export 和 Search 的 Session 来源 |
| 复发入口 | 合同生成器、退役门禁、脚本登记、AGENTS 和文档引用 |

## 已实现的新设计

生产链已有 canonical AgentSessionStore，和主业务库共享 SQLite。Session、Turn、Event、Payload、Effect、Resource、Agent Revision 与 Snapshot 分别拥有明确职责；Conversation API 是同一个 Session 的产品投影。旧 Conversation 表与 Nomi 私有文件 transcript 已退出生产主链。

`nomifun.nomi` 是唯一官方 Runtime。当前模型切换、Agent binding transition、原生暂停 checkpoint、效果未知与取消的安全条件属于这条新链，整改继续保留这些条件。

Message 用于查询与展示，模型历史应从 canonical 事实重建。领域消息通过正式事件进入上下文；Execution、监督器与渠道可以拥有业务状态，但不会因此获得另一套 Session 或 Turn authority。

## 确认的残留和整改

| 发现 | 基线代码证据 | 实际整改 |
| --- | --- | --- |
| 缺 Runtime records 或 TurnStarted 时整窗改读消息文本 | `unified_runtime_history::load` 返回 `None`；host 读取 Message history | 删除整窗 fallback 和 projection history API。缺失结构日志明确拒绝；正式未启动失败保留 accepted input 与失败事实 |
| 用 UI position 推断模型角色，缺 producer sequence 时补号 | `EngineHistoryMessage`、`project_messages`、history record 解码 | typed role、seq、content 从 canonical event payload 构建；sequence 必须由事实提供 |
| 两份 Runtime 事件解码实现 | `unified_runtime_history` 与 `runtime_history_port` | 两入口共用同一 decoder 和结构完整性判断 |
| 三代推理强度列，三写与逐级回退 | Store 的 `reasoning_effort`、`reasoning_effort_v2`、`reasoning_effort_v3` | Session 只持久化一个 native 字段；删除有损镜像、旧值回退和历史转换迁移 |
| 无生产调用的 generic checkpoint 和 RuntimeBinding 链 | Store bind/admit/discard API、head 六字段、event 两字段、runtime registry kinds | 删除 API、DTO、schema、producer 校验、切换 discard 分支及仅自测使用的夹具；保留实际 native checkpoint |
| 隔离 schema 与产品数据库不同 | 合同目录 34 表小 schema 包含旧 Plugin 表，Remote 表与主库列集合不同 | 删除小 schema；Contracts、Store 和 DB 统一引用完整 canonical baseline |
| fork 宣称自包含但 payload 只有父 ID 与 cursor | `canonical_session_owner::fork` 的 base body | 冻结 typed 数据正文、来源 binding、cursor、版本和 digest，写入 child-owned payload；恢复不查询父会话 |
| UI 依赖不存在的终态 marker，默认执行旧本地后处理 | `final_text_authoritative` 与 Nomi buffer/post-process/local Cron helpers；Rust 无相应生产字段 | 删除 marker、buffer 和兼容状态机；直接消费当前 stream 与回执 |
| 旧错误文字重分类与旧工具行读取过滤 | Messages hooks 与 plan/preflight visibility helper | 删除旧内容修补；当前 producer 负责语义，UI 使用明确 error code |
| ToolGroup 保留另一条旧产物通道 | UI 的 `result_display`、WriteFile 和图片展示；当前 Rust Group 只有 summary 四字段 | 删除旧通道；产物使用当前 ToolCall committed receipt 和 artifacts |
| 创建入口自动导入旧草稿 | legacy workbench draft import 与旧参数转换 | 删除 reader、调用、测试与旧会话标记；新 generation 不读取旧草稿 |
| 缺 Channel binding 时自动接回旧记录，改变 chat-kind | SQLite channel 的最早旧行回查、重分类和无 chat-kind wrapper | orphan binding 和 scope 变化报冲突；正式新建和重置走明确 owner 命令 |
| 无调用的旧 Execution cleanup 接口 | 旧宽入口与 exact cleanup 入口并存 | 删除无调用的旧入口，保留精确身份边界 |
| Agent 缓存沿用未变化的物理 storage marker | SystemInfo 与 browserStorageKey 未区分 Agent 数据代际 | Rust 单一代际值进入严格启动初始化，Agent 草稿、预览、队列与回执换命名空间；Terminal、Provider、主题偏好保留 |
| 生成器和门禁强制保留历史材料 | historical deletion manifests、D025 generic fixture、旧计划 required-files 与 archive 标记检查 | 物理删除旧合同、夹具、计划和归档依赖；生成物只使用当前合同 |

代码证据可通过 `git show 2ddf08dbc:<path>` 对照基线。现行实现入口集中在 [Session 架构文档](../architecture/agent-session.zh.md#实现入口与验证)，删除的文件不保留副本或跳转占位页。

## 数据基线与切换方案

本轮建立新数据代际和一份完整 canonical baseline，删除旧 forward migration SQL。Contracts 从这同一份 SQL 计算结构 manifest；schema SQL 不嵌入自身 digest，metadata 由唯一 Rust 合同初始化，避免两个 schema 源及摘要自引用。

启动边界只识别上一完整 canonical generation 的精确 migration receipt 指纹。指纹是切换准入凭据，不用于打开旧 Agent 日志。未知 checksum、部分 lineage 和不符合切换条件的库拒绝启动，不尝试修补或猜测。

允许切换时，在同一数据库事务中清空 Agent 数据及活动绑定，替换 Agent-owned 表和结构，更新 metadata 与新 baseline receipt。非 Agent 配置和业务数据原位保留，不通过第二数据库或会话转换器搬运。失败必须回滚；完成后再次启动只读取新基线。

Channel 的入站去重事实保留外部事件与 operation 身份，并清空旧 Agent 活动引用，防止已处理的 IM 消息重投递产生重复操作。它不恢复或读取旧会话。业务 Requirement、Provider、Model、Plugin、MCP、Knowledge 和应用配置按各自 owner 保留。

## 保留边界

以下内容有当前生产用途，不作为历史兼容删除：

- 同代前序 Turn、冻结 Snapshot、合法的 Agent 与模型 transition，以及 native checkpoint 的精确 build 校验。
- UI、Export、Search、Companion、Channel 和 Execution 的 Conversation DTO；它们均投影同一个 canonical Store。
- 当前 Provider 的 inline reasoning 输入适配、当前支持的外部协议及第三方数据 ID。它们不提供退役 Session 格式读取能力。
- 各领域自身的配置、外部消息去重、业务生命周期与监督决策。Session 输入、取消和终态仍引用 canonical Turn receipt。

## 实施顺序与完成标准

| 顺序 | 实施切片 | 完成标准 |
| --- | --- | --- |
| 1 | 删除 Runtime 的投影历史降级 | 新 Turn、准入失败后继续、切换后多 Turn、坏日志拒绝及唯一解码器测试 |
| 2 | 删除 UI 与领域旧内容兼容 | 当前 stream、history、artifact、暂停、只读、创建和 Channel binding 回归 |
| 3 | 删除 generic 恢复链与字段镜像 | 无生产/测试闲置 API、DTO、旧表列和 event kind；native 恢复仍安全 |
| 4 | 合并完整 schema 和原子 clean cut | fresh init、旧完整代际切换、非 Agent 保留、坏指纹拒绝、失败回滚与 reopen |
| 5 | 闭合 fork 内容 | 父删除后 child 和后续 fork 可恢复；清空上下文后继承正文消失；digest、预算与幂等校验 |
| 6 | 删除过期文档与生成依赖 | 当前文档不链接旧方案，生成器不读取历史材料；旧源仅在 Git 历史 |
| 7 | 固化退役边界 | AGENTS 指向当前架构；普通 check 强制新 Session 边界，退役引用预算为零 |

测试不以兼容 fallback 修成绿色。缺事件、坏序号、损坏内容和错误 scope 必须按明确错误处理，不能补造成功或权限。

## 验证记录

已通过的验证如下。测试前置仍依赖旧小 schema、旧字段或旧接口的用例已改为实际当前合同；没有通过恢复兼容实现使测试通过。

| 验证 | 结果 |
| --- | --- |
| 全部 UI 测试 | 合入远端任务计划重构后 3896 通过，0 失败 |
| Contracts、Session、DB library | 67、81、311 通过；新增严格 context snapshot 用例另 4 项通过 |
| API types library | 517 通过 |
| App library | 633 项通过；唯一失配的负例夹具修正后定向通过，2 项按原条件忽略 |
| Channel、Conversation、System library | 354、23、141 通过 |
| Native execution recovery、canonical routes、startup smoke | 7、41、4 通过 |
| DB reset、index/query-plan、native reasoning | 2、2、1 通过 |
| 普通 bun run check | 类型、i18n、主题、真实 CSS 生成器、桌面边界、Runtime/Browser/Plugin/Session 等全部通过 |
| App 全部 feature 编译 | cargo check --all-features 通过 |
| 退役边界 | 13 组生产引用均为零，新 Session 边界通过 |

切换测试明确覆盖 non-Agent 保留、旧 Agent 偏好清空、坏指纹和部分 lineage 拒绝，以及删除失败时数据、schema 和 receipt 一并回滚。Fork 测试覆盖父删除、父切 Agent、child rename 后原始幂等重放、不同请求冲突、跨 owner 拒读、内容篡改、预算与 clear context。

索引审查确认原完整发布库与新基线均有 116 项显式索引，名称、归属与定义一致；114 是旧初始基线的计数。保留原有每表预算与热点查询计划验证，没有扩大预算。

所有数据库验证使用隔离临时库；真实用户数据库未打开、重置或导入。本轮没有发布或启动用户数据目录中的新版本。新版本在满足精确切换条件的旧库上启动时，将执行已确认的 Agent-only clean cut。

macOS 原生、签名安装包、真实 Provider 和统计 99% 可靠性验收未运行。本 Windows 主机只提供本地、协议与自动化检查证据；平台状态仍明确保留未验证项。

## 提交整合验证

提交推送前合入远端 canonical 任务计划重构，保留其从同一 Event 与 Turn 事实读取的进度视图，并维持本轮旧状态机和字段删除。真实暂停状态以同一活动回合的 canonical head 投影为 paused，恢复和取消使用原有 native 生命周期，未新增状态存储。相关 Store 4 项测试、TaskPlan HTTP 重启回归、数据库切换与原生字段/索引 6 项测试及完整前端 3896 项测试通过，普通 check 和合同生成检查通过。
