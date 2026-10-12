# NomiFun 插件系统调研报告

日期：2026 年 10 月 9 日。代码基线：`cf440789160e43d94e0159640d817f22b17dc0f5`。验证环境：Windows，仓库当前工作区。

当前插件平台已经具备统一包合同、独立作品身份、持久存储、进程管理、创作验证和安装恢复机制，但这些机制在组合使用时仍有明显缺口。本次发现 15 项缺陷，其中 7 项列为 P1。最严重的问题涉及升级丢失已确认写入、Service 退出后宿主 SQL 继续提交、跨插件初始化锁等待，以及一个插件拖累其他作品或整个 Agent 会话启动。

优先修复顺序应是数据一致性与运行隔离，其次是创作验证、工具容量和产品状态。现有专项测试全部通过，却未覆盖这些关键场景；“测试通过”和“插件实际可靠”之间存在具体的验证缺口。

## 调研范围和证据

本报告审查当前“插件与小程序”平台，以及它与 Agent、桌面入口和前端的连接。源码主线包括 `nomifun-plugin-platform`、`nomifun-plugin-development`、Plugin Router、Agent Plugin Binding、UI Bridge、作品库、创作工作台和运行页。

仓库还存在 IM 渠道的 `ChannelPlugin`、Agent 内置能力注册和 Tauri 原生插件。它们属于不同层次，不能仅因都叫 Plugin 就认定为重复的安装系统。这里核对其与当前平台的边界，未逐一验收全部 IM 服务商实现或第三方 Tauri 库。

现役要求以[插件架构](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/docs/architecture/plugin-platform.zh.md)、[产品说明](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/docs/specs/2026-10-02-agent-plugin-authoring-redesign/README.zh.md)及执行源码为依据。全部 UI 检查遵循桌面产品边界；没有运行移动视口或手机仿真。

| 证据类型 | 含义 |
| --- | --- |
| A 运行复现 | 使用真实 Node、生产安装器、Storage、Service Runtime 或 Backup 对象，在隔离临时目录观察到问题 |
| B 组件复现 | 使用真实 React 组件和符合实际接口的模拟 DTO 验证界面行为，尚不等同 Tauri 完整端到端验收 |
| C 源码确认 | 追踪生产调用链、状态写入和错误处理，能够确定机制缺陷；未对该完整场景做运行复现 |

P1 表示应优先处理的数据损失、错误副作用、关键流程阻断或跨作品影响；P2 表示有明确触发条件的功能、合同或可诊断性问题。优先级不代替证据类型。

| 检查领域 | 已有机制 | 本次发现的缺口 |
| --- | --- | --- |
| 包和 Manifest | 统一合同、未知字段拒绝、路径与摘要验证 | Action 数量缺少与消费端容量匹配的准入约束 |
| 安装和升级 | CAS、作品 mutation mutex、staging、journal、失败回滚 | 日常写入不受升级切换保护，历史 migration 复核不完整 |
| 存储 | KV、SQLite、Files、Cache，各作品 DataRoot | SQL 取消与任务收尾不足，UI 与 Service 的接口不一致 |
| Service | 独立 Node 进程、onDemand、continuous、进程树清理 | 全局启动锁重入；进程结束不能证明宿主存储停止 |
| Surface 和 Bridge | iframe、MessageChannel、身份和 Artifact 校验 | 全局锁覆盖慢调用、SDK 注入顺序、失败撤销和不确定结果表达 |
| Agent 消费 | 冻结工具映射、启用状态和 Artifact 复核 | 全局工具上限阻断会话，超时错误误报可安全重试 |
| 创作和验收 | canonical Session、准确计划和验证报告 | UI runner 依赖面板状态，持久化证明可误通过 |
| 管理和交互 | 分类、常用、配置、源码编辑、运行、回收站 | GUI 保存状态错误，设置草稿被刷新覆盖 |
| 导出和恢复 | 标准 Package、Backup、回收站恢复 | 活跃数据库无法可靠备份，草稿 JSON 不能完整往返 |
| 桌面和 WebUI | 明确的本地信任与桌面准入 | WebUI 实际只读、若干 Host 能力尚未实现，需要准确表述 |

## 缺陷总览

| 编号 | 优先级 | 问题 | 证据 |
| --- | --- | --- | --- |
| F01 | P1 | 数据版本升级丢失迁移期间已确认的正式写入 | A |
| F02 | P1 | Action 超时和 Service 退出后，宿主 SQL 仍提交 | A |
| F03 | P1 | onDemand 初始化跨插件调用发生全局锁等待 | A |
| F04 | P1 | 慢 Action 持有全局 Surface 锁，阻塞其他作品 UI | C |
| F05 | P1 | 合法插件可耗尽工具上限，阻断普通 Agent 会话启动 | C |
| F06 | P1 | 有副作用的 Action 超时被标记为可安全重试 | C |
| F07 | P2 | 活跃 SQLite 的 WAL 文件使 Backup 导出失败 | A |
| F08 | P2 | 已执行 migration 被改写后仍允许更新安装 | A |
| F09 | P1 | 收起产物或选择另一作品会阻断 UI 验证和交付 | B、C |
| F10 | P2 | Service 持久化验收可以在没有持久化时通过 | C |
| F11 | P2 | GUI 保存安装成功，但界面仍显示创作中 | B、C |
| F12 | P2 | 无关插件变更会覆盖设置弹窗的未保存输入 | B |
| F13 | P2 | 无效配置或过期 revision 仍撤销已有 Surface | C |
| F14 | P2 | UI 与 Service 的数据库及 KV SDK 合同不一致 | C |
| F15 | P2 | SDK 注入晚于 head 同步脚本，初始化拿不到 API | DOM 复现、C |

### F01 数据版本升级丢失已经确认的写入

更新先复制活跃 DataRoot，再异步迁移和验证，最后才停止旧 Runtime。升级提交复制出的 generation 后，旧 generation 被删除。Plugin mutation mutex 串行化管理操作，没有覆盖日常 Storage 写入。

证据：[复制正式数据](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/install.rs:555)、[迁移](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/install.rs:582)、[停止旧运行时](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/install.rs:640)、[清理旧数据](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/install.rs:754)。

复现让真实 migration 在数据复制之后暂停，通过旧 DataRoot 的生产 KV adapter 写入并读取确认，再放行更新。结果：

```text
ACKNOWLEDGED_WRITE_REVISION=1
VALUE_AFTER_UPGRADE=None
OLD_GENERATION_EXISTS=false
```

这是已复现的数据丢失。范围是切换数据 generation 的升级；同 dataVersion 更新继续使用原 generation，不能将此结论扩展为所有代码更新都会丢数据。HTTP 更新路径会提前撤销已有 UI Surface，但后台 Service 和已经进入的宿主存储操作仍有这个窗口。

修复应在生成最终快照前冻结正式写入，等待已进入操作结束，再复制、迁移和切换；失败时恢复准入。若希望迁移期间继续接受写入，需要完整的变更追赶机制，单靠管理 mutex 不够。

### F02 进程退出后宿主 SQL 仍然提交

Service 的存储请求由 Rust `spawn_blocking` 执行。请求超时只取消 token 并停止等待，进程终止也没有等待这些宿主任务完成；SQLite 没有执行中断或进度取消机制。`busy_timeout` 只限制等锁时间，不能限制 SQL 运算时间。

证据：[宿主超时处理](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/service_process.rs:1403)、[存储线程任务](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/service_process.rs:1443)、[进程结束清理](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/service_process.rs:973)、[数据库执行](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/data_root.rs:1060)。

复现仅把 Process Factory 请求超时缩短为 50ms，使用真实 Node、Host 和 SQLite 执行有限计算量的 INSERT：

```sql
WITH RECURSIVE seq(x) AS (
  VALUES(1)
  UNION ALL SELECT x+1 FROM seq WHERE x < 10000000
)
INSERT INTO audit_late_commit(total) SELECT sum(x) FROM seq
```

```text
SQL_ACTION_RESULT=Err(TimedOut)
ROWS_AFTER_PROCESS_TERMINAL=0
ROWS_LATER=1
PROCESS_TERMINAL=Some(Err(Crashed("service action timed out")))
```

Service 已进入终态，Rust 线程随后才提交数据。生产默认 30 秒超时存在同一机制；复现没有声称普通 SQL 都会超过该默认值。该缺口使停止、停用、更新和取消所依赖的写入切点不可靠。

修复应给 SQLite 运算设置取消或执行预算，并让 quiesce 等待已接收的 Host Storage 操作结束。仅证明 Node 进程树退出，无法证明插件的全部效果已结束。

### F03 onDemand 初始化跨插件调用导致锁等待

`PluginServiceRuntime::invoke` 持有所有插件共用的 `slots` mutex，等待进程启动和 Hello；Service 在 `activate(ctx)` 完成后才发送 Hello。若 activate 等待另一个插件的 Action，回调又进入同一个 Runtime，需要重新取得这把锁。

证据：[持锁启动](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/service_runtime.rs:87)、[activate 与 Hello 顺序](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/service_process.rs:194)、[生产 Action 回调](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin_ports.rs:82)。

真实运行时探针先直接调用 B 成功，再调用 A；A 的 activate 执行 `await ctx.actions.invoke("plugin:<B>/ping", {})`。actions port 重现生产回调至 Runtime 的链路：

```text
CROSS_PLUGIN_STARTUP_RESULT=Err(TimedOut)
ELAPSED_MS=10056
HOST_ACTION_PORT_CALLS=1
```

A 等 B，B 等 A 持有的全局锁，直到默认 10 秒 Hello 超时解除。这个等待也拖延其他插件取得 Runtime 锁。它不是所有跨插件 Action 都失败：问题集中在持锁启动阶段。

修复应把启动同步细化到作品级别，在等待进程或作者代码前释放全局 map 锁，并保留单实例启动和 generation 校验。

### F04 慢 Action 阻塞所有 Surface

`dispatch_bridge` 取得唯一 `state.surfaces` mutex 后，在持锁期间等待 Action 完成，随后更新该 session 的 completed calls 才释放锁。所有作品的 Bridge、资源读取、Surface 打开、关闭和撤销都使用同一把锁。

证据：[取得全局锁](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:1903)、[持锁等待 Action](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:1942)、[关闭也依赖该锁](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:1776)。

触发方式：作品 A 从 UI 调用一个慢 Service Action，同时在 B 执行 KV 读取、打开页面或关闭 Surface。B 会排在 A 后面，多个慢调用还会形成排队和 SDK 连锁超时。

此处有默认 30 秒 Action 超时及短暂取消等待，不能描述为永久死锁，也不能说所有 Agent 或 Service 调用都停摆。影响的是共用这把锁的 Surface 路径。UI 的 SQL 路径还在持锁期间同步执行计算，进一步放大影响。

修复应在短临界区取得可独立使用的 session 上下文，然后释放 map 锁执行操作，最后按 session 身份、generation 和撤销状态提交结果。

### F05 插件工具数量可以阻断普通会话启动

非 restricted Session 会合并全部启用的 `agent.tool` 插件工具。合并 Agent、MCP 等工具后，消费端若超过 128 个 Action，直接拒绝构建 Nomi Runtime；延迟展示投影发生在上限检查之后。Manifest 与安装启用链没有对应容量准入。

证据：[冻结全部可用插件工具](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/engine_plugin_bindings.rs:124)、[合并后直接拒绝 Runtime](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/unified_runtime_host.rs:347)、[Manifest Action 校验](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-agent-contracts/src/plugin.rs:252)。

触发方式：安装一个含 129 个合法 agent.tool Action 的插件，或多个插件与已有 Agent、MCP 累计超过 128。插件本身可以安装启用，普通新会话或重建 Runtime 随后报 `Nomi tool surface exceeds 128 actions`，插件创作会话也可能受影响。已有缓存 Runtime 可能暂时继续工作。

这是源码确认的容量冲突，未实际安装 129 Action 包验证完整会话。修复应让插件按 Session 选择或通过可控的动态工具目录接入，并在安装、启用时提供容量诊断，保证普通聊天仍可启动和恢复。

### F06 超时被误报为可以安全重试

Agent 插件工具错误投影对所有 `PluginBindingError::Timeout` 输出 `retry_safe=true`，没有使用已经冻结的 read、write、external 效果类别。超时只发出取消并短暂等待，没有证明业务未执行、已经回滚或外部服务未提交。

证据：[向模型返回 retry_safe](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/engine_plugin_bindings.rs:401)、[超时取消路径](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/bindings.rs:643)。

若 external Action 已发送消息或提交订单，随后等结果超时，模型仍得到“安全重试”，再次调用可能产生重复副作用。该后果为源码机制推导，未向真实外部服务发送请求验证。

相同的不确定性在 UI Bridge 也被削弱：[Router 一律返回通用错误和 outcome_unknown=false](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:1982)；[Surface 对普通网络异常也返回 false](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/ui/src/renderer/pages/plugins/PluginSurfacePanel.tsx:45)。响应丢失不能证明写入没有生效。

对已经派发的有副作用调用，超时、取消或响应丢失应保留结果不确定性；准入前取消、Schema 或权限拒绝可以证明未执行。只有得到未执行证明或具备可靠幂等协议时，才允许有副作用调用安全重试；同时保留原始权限、Schema、Artifact、取消和超时错误类别，供模型及界面正确恢复。

### F07 活跃 WAL 使备份失败

正式数据库使用 WAL 模式，但导出 Backup 直接扫描活跃 generation，仅接受 `data.sqlite` 和 `files/**`，把正常 SQLite sidecar 当作非法内容。

证据：[直接传入活跃 generation](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:1391)、[目录扫描](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/backup.rs:301)、[拒绝 sidecar](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/backup.rs:578)、[数据库 WAL 模式](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/data_root.rs:1397)。

保留真实 SQLite 连接、提交记录后，调用生产备份器得到：

```text
LIVE_WAL_BACKUP_RESULT=Err(UnsupportedEntry("data.sqlite-shm"))
```

已确认的行为是备份失败，不能表述为“已经成功导出一个漏数据的 Backup”。不过修复不能只是过滤 WAL 文件：已提交数据可能仍在 WAL 中，裸复制 main 文件无法建立一致备份，数据库与 files 也需要同一备份切点。

修复应使用 SQLite 一致快照和临时完整备份目录，再原子导出；同步处理与升级、Files 写入之间的并发关系。

### F08 历史 migration 不可改写合同没有全面执行

已记录 migration 的摘要校验只在执行单个 migration 时发生。同 dataVersion 返回空迁移链，升级也只挑选当前版本之后的步骤，所以历史脚本不会进入该校验。

证据：[同版本空链及筛选](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/install.rs:1432)、[单个 migration 内的摘要复核](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/service_runtime.rs:291)。

真实探针先执行并记录 v0→v1，再保持 dataVersion=1 和同一 migration ID，改写脚本后更新安装：

```text
REWRITTEN_ALREADY_RECORDED_MIGRATION_ACCEPTED=true
```

此行为违反架构中“已记录的 migration 被改写时拒绝”的明确合同，可使全新安装和升级实例经历不同的数据变换。修复应在选择待执行步骤前，复核新包中与历史记录对应的 migration ID、版本边和文件摘要。

### F09 UI 验证依赖产物面板展开和选中状态

唯一的 UI 测试执行器在 `PluginSurfacePanel` 中。产物组件只有 `visible && selected` 才 active，非 active 就不提供 descriptor，还会关闭原预览。后端只等待 UI results，25 秒后返回 Surface 未响应。

证据：[选中与可见状态决定 active](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/ui/src/renderer/pages/plugins/PluginAuthoringArtifacts.tsx:69)、[不挂载非 active Surface](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/ui/src/renderer/pages/plugins/PluginAuthoringArtifacts.tsx:149)、[后端等待前端验证](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin_authoring.rs:469)。

两个真实组件探针均确认问题：创作时收起产物面板，随后的 UI case 没有 runner；同任务有两个 UI 作品，用户选中第一份，第二份已有 pending case 和 descriptor，却没有 runner。后端超时机制由源码确认。

这把用户正常的布局操作变成创作正确性的前提，多产物交付尤其容易受阻。修复应让验证执行器由 canonical 创作 Session 和 pending command 决定，面板是否展开、用户查看哪份作品只影响展示。

### F10 Service 持久化证明可以误通过

计划验证对 Service 持久化只要求关联 Action case 的 `restart=true`。执行测试在调用 Action 之前重启 Preview；只要这次调用结果符合预期，就记录 `persistence_checked = restart && passed`。

证据：[计划只检查 restart 标记](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-development/src/lib.rs:309)、[先重启再调用](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin_development.rs:419)、[写入持久化证明](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin_development.rs:447)。

最小反例：`put(value)` 只写进程变量并返回 value。计划仅测试 put，设置 restart=true 和 requires_persistence=true。测试会先重启再写变量，因此结果正确并取得持久化证明，但下次进程退出数据丢失。

作者 Guide 提醒执行“写入后重启读取”，然而机器校验没有强制这个顺序。此项为源码确认，未运行完整创作交付反例。修复应把写入和重启后读取绑定为同一份受验证证据，不能以重启后任意 Action 成功替代数据恢复证明。

### F11 手动保存成功却仍显示创作中

GUI `/save` 实际安装包并关联草稿，未产生 Agent 专属 `verification.delivery`。但 UI 用从该字段读取的 `delivered_artifact_digest` 判断通用“已保存”、是否提供打开按钮，以及是否使用正式 Surface。

证据：[GUI 保存](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:1210)、[关联草稿的 SQL](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/repository.rs:585)、[摘要只读 delivery](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:2442)、[UI 状态判定](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/ui/src/renderer/pages/plugins/PluginAuthoringArtifacts.tsx:146)。

触发方式：手动修改已交付作品源码，旧 delivery 被正确失效；点击“保存并使用”后包已安装，界面仍显示创作中、没有打开入口，无改动还可重复保存，再预览仍走临时草稿。

符合实际返回 DTO 的组件探针已复现。原有保存测试 mock 人为填写 saved digest，掩盖了这个后端与 UI 的合同错位。

修复应从 save 返回的正式插件和源码关联识别安装成功，同时保留“Agent 已完成交付验收”的独立含义；不要为了补界面状态而伪造交付验证证据或另建事实账本。

### F12 设置弹窗未保存内容被刷新覆盖

RunPage 收到全局 plugins.changed 就重新读取当前作品并替换 detail 对象，即使其内容和 revision 未变。配置弹窗初始化 effect 依赖整个 detail 对象，所以一次无关刷新会重设配置、凭据选择和 grants。

证据：[全局事件刷新 detail](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/ui/src/renderer/pages/plugins/PluginRunPage.tsx:87)、[detail 变化时重置表单](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/ui/src/renderer/pages/plugins/PluginConfigurationDialog.tsx:38)。

真实组件复现：输入 `{"duration":45,"theme":"violet"}`，暂不保存；返回相同持久内容的新 detail 对象并触发其他插件变更事件。输入恢复为原来的 `{"duration":25}`。

修复应按弹窗打开和作品身份初始化，保留 dirty 输入。作品实际被并发修改时，再显示冲突和重新加载选择。

### F13 失败配置请求仍撤销已有 Surface

配置入口在读取 inventory、Schema 校验和 CAS 前就撤销所有关联 Surface。之后若配置非法、凭据无效或 revision 过期，请求失败，但旧 session 已删除，安装器的 Runtime 回滚无法重建 Router session。

证据：[先撤销后校验](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:1494)、[移除所有关联 session](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:1732)、[配置 CAS](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/install.rs:933)。

准确范围是：无效请求仍使其他已有同作品 Surface 或关联 Preview 失效，正式配置和 revision 却没有成功修改。普通 RunPage 在发送配置前也会主动关闭自己的 Surface，因此不能把该窗口所有关闭行为都归因于后端。

修复应先完成输入与 revision 预检，再在与作品 mutation 协调的切点撤销运行访问，并定义失败后的恢复行为；同时覆盖多个 Surface 和在途打开请求。

### F14 UI 与 Service 的 SDK 数据合同不一致

相同 `storage.db.query` 在 UI 返回按列名映射的对象行，在 Service 返回 `columns` 加二维数组。BLOB 在 UI 作为 base64 字符串输出，Service 使用 `{bytesHex}`；UI SQL 参数拒绝对象，因此也不能接受 Service 的 BLOB 参数表达。

证据：[UI 查询结果](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:2566)、[UI SQL 参数](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:2555)、[Service 查询结果与 BLOB](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/service_process.rs:1669)。

例如 `SELECT body FROM items` 的两种结果是：

```json
{"rows":[{"body":"persisted"}]}
```

```json
{"columns":["body"],"rows":[["persisted"]]}
```

KV 的能力也不对齐：Service 有读取 value 与 revision 的 `kv.read`，UI 只有丢弃 revision 的 get，但仍提供要求 expectedRevision 的 compareAndSwap。UI 可通过失败 CAS 等绕行获取 revision，却缺少常规的原子读取 value 和 revision 接口。证据：[Service kv.read](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/service_process.rs:145)、[UI SDK API](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-plugin-platform/src/assets/plugin-sdk.js:463)。

因此混合插件无法直接共用数据库访问 helper，UI 并发更新也缺少自然的 CAS 用法。修复应统一公共结果和二进制合同，补齐 UI revision 读取，并对同一数据在 UI 与 Service 的往返建立互操作验证。

### F15 SDK 注入晚于作者同步脚本

注入器把 SDK 插到 `</head>` 前。head 内已有的普通同步脚本先执行；若它立即读取或调用 `window.nomi`，API 尚未建立。作者合同没有禁止这种合法 HTML 初始化方式。

证据：[SDK 注入位置](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:2754)。

Node 加 HappyDOM 解析 HTML 并加载完整生产 SDK，观察：

```json
{"headAuthorScriptSaw":"undefined","afterInjection":"object","errors":[]}
```

探针读取类型，没有主动制造异常；真实脚本在此时调用 API 会失败或捕获 undefined 引用。body 脚本、defer、module 或 DOMContentLoaded 初始化能避开，所以不能说全部 UI 插件都会启动失败。

修复应在任何作者脚本前可靠建立 SDK，或者提供明确且由 Host 强制执行的初始化合同。

## 产品限制和需要继续验证的风险

以下不计入上述 15 项已确认缺陷，也不能直接当作权限漏洞。

| 项目 | 当前事实与影响 | 后续处理 |
| --- | --- | --- |
| Preview 可影响其他正式作品 | 跨插件调用仍需 actions.invoke 授权，但 dispatcher 使用正式 registry；临时 DataRoot 只隔离本作品数据，不能隔离其他作品或外部副作用 | 明确 Preview 语义，提供可控的写调用隔离策略；核验测试调用造成的真实影响 |
| 在途 Surface 打开与撤销竞争 | open_surface 只校验一次 inventory，随后才插入 session；storage dispatch 不再次校验当前启用状态或 revision | 用确定性 barrier 测试停用、配置、更新与 open 并发，检查是否会插入旧权限 session。当前只是待复现风险 |
| 草稿导出不支持完整导入往返 | 创作页下载 files JSON；二进制文件 detail 没有内容，仅路径、摘要、大小。标准导入只接受目录、ZIP、Backup | 说明这是源码快照，或提供保留二进制资源的完整包导出；成品页 export 已有完整包路径 |
| 桌面 WebUI 实际只读 | 现有 UI 和边界测试明确限制创作、导入、Surface、配置和 Action 运行；“支持 WebUI”不能理解为与 Tauri 功能一致 | 在产品说明和页面提示中写清可用范围。若改变范围，需要显式产品决定与本地信任方案 |
| Node 是本机代码而非权限沙箱 | 文档明确 permissions 控制 Host Bridge 与 UI 网络，不限制 Node 原生文件、网络、子进程 | 根据实际信任模型说明权限含义，不能宣传为操作系统级隔离 |
| 新声明权限默认授予 | 安装与权限扩张没有独立审批对象，这是当前产品合同 | 如需改变，应作为产品决策；本次不把缺少审批流程误报为 bug |
| desktop.files.open 未实现 | 生产 Host 返回 capability unavailable，作者 Guide 已明确提示 | 不应生成依赖该能力的可用功能；补实现或准确呈现能力可用性 |
| automation.action 尚未接调度器 | 只有通用 registry owner，不等于计划任务已可使用插件 Action | 明确支持层次，接入调度器后补消费端测试 |
| 旧 Session 工具冻结 | 更新后旧映射返回 Artifact changed；新安装不会自动进入旧 Runtime。当前会话用新工具有专门安全重建流程 | 提供可靠的工具刷新入口，不能只提示重开页面；保留明确 Artifact fence |
| Context 和 Hook 总时间预算 | 多个绑定串行执行，每项可等待 30 秒，缺少整体共享时限 | 做多慢插件压力验证，设置回合级预算和可诊断的降级策略 |
| 错误提示信息不足 | 导入、配置和部分运行操作把明确后端原因换成通用提示；Bridge 也丢失细分错误 | 把 Schema 路径、冲突、迁移和能力不可用原因转为可操作提示 |

相关源码：[Preview Action dispatcher](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin_ports.rs:67)、[Surface 准入](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin.rs:1651)、[草稿导出](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/ui/src/renderer/pages/plugins/PluginAuthoringArtifacts.tsx:237)、[WebUI 边界测试](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/ui/src/renderer/pages/plugins/PluginPlatformBoundary.test.ts:6)、[Binding 时限](C:/Users/rika0/code/nomifun/bak/mobile/nomifun-desktop/crates/backend/nomifun-app/src/router/plugin_ports.rs:148)。

## 修复顺序和验收条件

| 顺序 | 工作内容 | 必须满足的结果 |
| --- | --- | --- |
| 1 | 数据升级切点、Host Storage 取消与任务收尾、备份快照 | 已返回成功的写入跨升级保留；quiesce 完成后不再提交旧写入；活跃 WAL 备份成功并能恢复准确内容 |
| 2 | 作品级 Service 启动同步和 Surface 并发 | A 初始化调用 B 成功；A 慢 Action 不拖延 B 的 KV、打开、关闭；撤销阻止迟到 session 恢复访问 |
| 3 | Agent 工具容量和不确定效果处理 | 超大插件不能阻断普通聊天；write/external 超时不误报安全重试，重试具有明确幂等依据 |
| 4 | 创作验证执行器和持久化证据 | 收起面板、切换作品、多产物均完成真实 UI case；只写内存的 put 不能通过持久化验收 |
| 5 | 手动保存与配置状态 | GUI 安装成功能直接打开正式作品；无关刷新保留 dirty 配置；失败配置不额外撤销其他 Surface |
| 6 | 历史 migration、SDK 合同和启动顺序 | 已执行 migration 改写被拒绝；UI 与 Service 数据合同可互操作；普通 head 同步脚本得到 SDK |

修复应延续现有 Plugin、Artifact、DataRoot 和 canonical AgentSession 事实链，避免为补界面状态新增第二套安装、交付或会话账本。并发修复必须同时处理准入、已进入操作、取消、提交和失败恢复，单独增加 mutex 或修改错误文案不足以闭合这些行为。

## 验证记录

现有专项测试共 150 项通过，统一插件边界检查通过：

| 命令 | 结果 |
| --- | --- |
| `bun run check:unified-plugin-boundary` | 通过 |
| `bun run test:plugin-sdk` | 14 通过 |
| `cargo test -p nomifun-plugin-platform --tests --offline` | 63 通过，包含 lib、artifact、storage、draft、install、migration、lifecycle、binding、backup、Service |
| `cargo test -p nomifun-plugin-development --test contracts --offline` | 3 通过 |
| `bun test --cwd ui ./src/renderer/pages/plugins ./src/common/adapter/pluginPlatformBridge.test.ts ./src/renderer/components/layout/Sider/pluginPlatformNav.structure.test.ts` | 70 通过，15 文件，296 assertions |

此外完成 5 个真实后端负面探针、4 个真实 UI 组件负面探针和 1 个完整 SDK 的 DOM 注入探针。探针“通过”表示成功观察并断言上述缺陷，未表示缺陷已修复。所有临时探针已移除，生产源码和原有测试未修改。

已有测试证明的有效保护包括：严格包路径和摘要校验、跨作品存储边界、保留表与 ATTACH 限制、配置与 Credential 引用分离、管理操作 CAS 和恢复 journal、Artifact fence、回收站和正式数据保留、独立 Service 进程与子进程清理、真实 SDK handshake，以及 canonical 创作 Session 和当前会话使用证据。发现的问题集中于这些机制之间的连接和边界，不应把正常保护一并否定。

当前验证未包含 Tauri 完整真机流程、真实模型生成、真实外部账号操作、全量仓库测试或跨操作系统验收。UI 组件探针使用模拟 HTTP DTO；容量冲突、错误重试和验收反例中的完整用户场景属于源码确认。这些限制已体现在各项证据等级中。

报告仅新增本文件，没有实施修复、提交或发布。后续改动应按影响运行针对性测试；涉及 renderer 时补 Desktop UI Boundary，涉及 canonical Session 内容时遵循 Agent Session 架构及边界检查。
