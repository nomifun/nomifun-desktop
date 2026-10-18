# 统一伙伴 Agent 本地功能验收

本页是本专项唯一的状态和证据索引。当前合同见
[伙伴架构](../architecture/companion.zh.md)、[Agent Session](../architecture/agent-session.zh.md)
和[语音架构](../architecture/voice.zh.md)。实施始于 2026-10-18，本地验收于 2026-10-19 完成。

## 验收结论与范围

用户先确认“实体设备暂不可用，先完成本地验收”，随后明确“不需要模拟现场 P50/P95，整体测试功能可用通过即可”。
按这一调整后的口径，本专项的源码、配置、UI、生成合同、必要回归、本地构建和隔离真实后端进程验收已完成。
本轮不要求或模拟现场分位数，也不据本地受控模型推导真实供应商或硬件的时延、成功率及提升百分比。

起始分支为 `main`，HEAD 为 `5beb4c90400403dc67f963d148361c7ed71824ad`，起始工作区干净。
验收完成时改动保持可审查，尚未 commit、push、发布安装包、安装新版本、刷写固件或清空用户资料。
真实进程测试全部使用本次新建的隔离数据目录，没有改动已安装实例的数据。

| 阶段 | 结果 |
| --- | --- |
| 代码、配置、执行和语音调用链复核 | 完成 |
| 统一定义、正式 transition、直接工作、通用后台、设备按需装配和生产语音 | 完成，受影响回归通过 |
| UI、生成合同、双语指南和当前架构同步 | 完成，桌面组件浏览器验证与合同检查通过 |
| 功能回归、完整检查、本地构建和真实后端进程验收 | 完成，证据如下 |

## 调用链和关键决定

旧的普通设备查询经过 Robot 模型委派、独立桌面 Attempt Session 的模型与工具循环、canonical 报告、
原 Robot Session 的隐藏结果 Turn 和再次模型总结。定向委派原本已跳过 Planner；不把它计作必然增加的阶段。
结果 Turn 未立即完成时，退避恢复可能附加等待，并非每次固定等待 60 秒。

现在普通工作在原入口的独立 canonical Session 内，直接使用同一伙伴冻结 Agent 的已授权工具。
通用 AgentExecution 保留给确有后台、隔离或并行收益的任务。后台携带原 accepted input、补充约束和确切工具 Effect 引用，
报告直接投影并沿原来源交付；普通结果不再固定启动模型总结 Turn。成功结果由事件推进，持久恢复仍处理丢通知及中断。

- 每个伙伴使用一套主 Agent、默认主模型、人格、记忆及 Skill/MCP 选择；ASR、TTS 等专用媒体配置独立。
  `companion.robot` 官方定义、第二个 `robot/<id>` 选择目标和 `target_role=desktop` 产品跳板已退役。
- 多窗口、设备和后台保留独立 Session 及明确输入目标。现存设备 Session 通过 idle/CAS/Effect/native cleanup 的正式 binding transition 进入统一定义，
  不改写 opening、旧 Payload 或历史事实，也不自动扩张个人作者授权。配置转换严格拒绝重复 key、异常结构及备份冲突，并保留原配置备份。
- 能力授权、工具展示、上下文预算及资源装配分离。只有确切授权设备的会话装配设备工具；未绑定不请求设备目录，离线不阻断普通工作。
  高频工具直接可用，大型 schema、Skill 正文和知识内容按需展开；描述和校验缓存按版本及内容区分，派发时仍核验连接和撤销。
- 物理语音消费正式、具有来源证明的公开提交。工具已受理后的不可撤回进度可在整个 Turn 完成前交付；最终答复仍须完整消息封口和成功 Turn 证明。
  Deepgram 增量 ASR 与 OpenAI PCM 流式 TTS 使用真实协议；不具备该能力的供应商明确走 Batch。
  保留节拍、背压、独立媒体 owner、队列排空、停播和精确 generation 隔离。
- 直接澄清与后台问题均使用 typed canonical 路径。排队 ACK 不代表问题已呈现；实际 writer 接受时才写呈现事实，
  并在短事务中检查原 Execution/Step/Attempt/Turn/native 与请求来源。新请求、补充、改口、停播、取消工作分别处理；
  录音开始即冻结目标，ASR 返回时不挑选最新任务。访客权限收缩和 unknown Effect 的保守恢复继续生效。

## 回归、检查与构建

以下记录实际运行范围，不把已注册但未执行的测试算作通过。

| 范围 | 结果与证据 |
| --- | --- |
| Companion / Agent Contracts | 全库分别 352/352、117/117 通过 |
| Agent Control Plane | 全库 96/96 通过 |
| Agent Execution | 全库 131/131 通过，`.tmp/companion-unified-execution-final.log` |
| Agent Session | 全库 299/299 通过，含新增同事务问题 writer/载荷 reader；`.tmp/companion-unified-session-final.log` |
| Agent Runtime | 全库 483 通过、1 忽略、0 失败。忽略项需要调用方提供仓库外历史 journal，未伪造历史输入 |
| Robot | 全库 228/228 通过；`.tmp/companion-unified-robot.log` |
| Model Invoke | 流式协议 7/7、OpenAI audio 19/19、manifest 34/34 通过 |
| App 编译 | lib 和 4 个相关 integration target 的 `--no-run` 通过；lib 注册 1137 项，未宣称全量实跑 |
| App 定向回归 | 按完整测试名去重的 172 个用例均有最终通过证据，覆盖工具装配、直接工作、背景工作、问题交接、精确取消、历史、输入生命周期及访客边界；分轮说明见下文 |
| App 集成 | 10 个不同用例通过；配置 6 项的末轮结果见 `verification-3/config/`，其余 4 项见 `verification-2/integration/` |
| UI | 主分组 61 项、相关分组 141 项、Execution 投影 4 项等定向测试通过；分组有重叠，不累加成唯一总数 |
| 生成合同 | 正式 `agent-v2-contract` 与 `voice-contract` 生成、末轮 `check` 均通过；UI API 版本为 40，未改写独立 Mobile 生成物 |
| `bun run check` | 末轮 exit 0；含 typecheck、i18n、桌面边界、Agent Session、UARC 及其他仓库必需检查。日志 `.tmp/companion-unified-check.log` |
| `bun run check:voice-boundary` | 末轮 exit 0，`.tmp/companion-unified-voice-boundary.log` |
| 最终文档与差异 | `bun run check:docs` exit 0，192 文件、1363 个本地链接和图片；`git diff --check` 通过，新源码无尾随空白 |
| `bun run build:ui` | exit 0；保留混合静态/动态导入与 chunk 大小警告，`.tmp/companion-unified-ui-build.log` |
| 标准 `nomicore` binary | `cargo build --locked -p nomifun-app --no-default-features --features browser-use,computer-use --bin nomicore` exit 0；`.tmp/companion-unified-nomicore-build.log` |

App 证据位于 `.tmp/companion-unified-app-tests/`。21 个定向分组中，末轮前 19 组通过；
另两组在修正联调问题及过时夹具后复验：shared context 19/19 通过；directed flow 的末轮全组 18/19 通过，
剩余的后台问题用例在修正夹具 ACK 顺序后，重新编译并单项 1/1 通过。因此 19 个不同 flow 场景均有通过证据，
没有将这两次执行描述为一次全组 19/19。额外 guest 5/5、input intake 2/2 通过。
最终日志分别为 `verification-3/history.log`、`verification-3/flow.log`、`verification-3/question-queue-order.log`，
以及 `verification-2/config-guest.log` 和 `verification-2/config-input-intake.log`。
按完整测试名去重的 172 项 App 单元与 10 项集成，共 182 项的最终状态、原失败和覆盖重跑关系，
见 `.tmp/companion-unified-app-tests/acceptance-summary.json`；没有零用例选择冒充通过。

这些场景实际经过正式 Host 和 canonical 事实链，覆盖直接天气搜索、真实文件写入/读回、真实原生进程输出、
按需知识搜索/读取及写回、多动作、设备会话复用、后台继续交谈、直接与后台口头问题、自然回答、补充、改口、取消和后继隔离。
受控 HTTP 搜索返回夹具内容；天气用例不证明真实气象。文件、进程和工具结果使用实际回执，不能只凭可见工具目录验收。

浏览器证据在 `.tmp/companion-unified-ui/browser-report.json` 与同目录截图。
生产 toolbar、Agent/模型/资源设置使用页面内模拟 IPC，聊天正文和草稿存储为夹具。
880×600、1280×800、浅/深主题均显示一个 Agent 选择器，无横向溢出；导航保留浮窗接收目标与原草稿。
Arco/React 19 的 `element.ref` 警告仍在日志中；该验证不宣称实际后端或设备 UI 已完成现场验收。

## 隔离真实后端进程

使用最新标准构建 `target/debug/nomicore.exe`，SHA-256：
`67ee63b03c676ae33c94a8964d22ad9acd73124e425bc1cc67d7f66954d294eb`。
后端实际以 PID 14020 监听 `127.0.0.1:61795`，health 为 `0.8.14`、build time `0`。
进程、HTTP、文件 owner 和 loopback WebSocket 均真实运行；模型与设备协议端为明确的本地受控夹具。
输入从 owner HTTP 进入设备归属 Session，不来自实体麦克风。

本轮证据目录为：
`.tmp/companion-unified-process/runs/2026-10-18T18-00-29-045Z-49b8ce37-f0ad-434e-859e-e7634daeb58a/`。
主要文件为 `process-report.json`、`nomicore.log`、`cleanup-verification.json` 和 `file-artifact-verification-all-run.json`。

实际结果：

1. HTTP catalog 中唯一伙伴官方定义为 `companion.default`，第二个 Robot 选择目标返回 422；统一 profile 不再含第二份 Robot 配置。
2. 桌面接收会话、独立主题、设备会话形成 3 条独立 Session，共享主 Agent/模型/能力；设备会话具备确切设备资源，切换主题不改桌面接收目标。
3. 设备 Session 实际写入并读回 55 字节文件，模型消费真实 read ToolResult。磁盘文件保存在本次 fresh runRoot 的 Companion 自有 workspace，关闭后仍存在且摘要一致。
4. canonical 记录为 1 个 completed Turn、2 个工具回执和 1 个 returned write Effect；Session 数前后均为 3，Execution 数为 0，没有隐藏结果回合。
5. Ctrl+C 后原生进程 exit 0，graceful teardown 日志、协议端关闭、PID 消失和端口无 listener 均已复核。
   外层 PowerShell PTY 因共享控制台中断返回 1，原生退出与外层控制结果分别保留，不抹去该区别。

该真实进程内，实际本地模型请求为 3 次，记录如下。它们属于同一个直接工作 Turn，没有额外分类模型或结果总结 Turn。

| 调用 | 可见工具数 | Tools JSON bytes | Messages JSON bytes | Request JSON bytes |
| --- | ---: | ---: | ---: | ---: |
| 1 | 11 | 16866 | 39719 | 56732 |
| 2 | 13 | 19679 | 41063 | 60889 |
| 3 | 15 | 30817 | 51677 | 82641 |

受控 flow 的直接天气场景为 2 次模型请求，首轮可见 14 个工具，Tools JSON 18756 bytes、Messages JSON 39391 bytes、
Request JSON 58311 bytes。新建设备 canonical Session 的一次本地观测为 4255037 μs，桌面为 3308434 μs；
仅计 `ensure_session_for_role` 调用，不含夹具 App 启动，也未独立测 Runtime 准备。这是单次本地初始化成本，不是现场分位数或产品响应保证。
受控知识按需用例实际调用 `knowledge/search` 与 `knowledge/read`，3 次模型请求，首轮请求 19361 bytes，普通装配不预先检索知识。

## 联调修复及保留边界

最终回归还修复了以下真实接缝，而没有放宽授权或完成合同：

- Host 来源字段曾误传给严格客户端 DTO，使物理输入停在 accepted；现以独立 typed intake 解析，原事件保持不变。
- cancelled/blocked 输入在没有 claim 时曾持续 pending；现使用 typed unavailable 状态与真实 event sequence，取消保持吸收态。
- 问题曾在队列 ACK 时就记录呈现，无法证明实际 writer；现由 writer 的短事务及 callback 建立确切呈现事实，取消/回答/改绑使后续 writer 失效。
- 访客渠道曾只检查旧 operation key，可能在权限收缩后重新受理旧请求；现沿原 accepted claim 核验当前权限与 context floor，owner 的原回执和历史仍可读。
- 保存结果不再取 projection 中的“最后一条 assistant”；现读取精确 canonical 封口来源。已退役隐藏结果仅有严格只读历史识别，不恢复新建或执行路径。

测试夹具也同步当前合同：只读工具使用真实 ToolCompleted，不虚构 mutation Effect；补充后先更新计划再工作；
正式完成使用 `report_completion`；问题及时 ACK 队列、只保持 writer 闸门；历史 DTO 水位与不可变 Event/Payload 分别断言。
早轮失败日志保留用于复核，当前通过状态以本文列出的末轮或更精确重跑为准。

真实供应商端到端 ASR/TTS、实体麦克风、实际播放、机械动作完成、真实多设备网络、正式安装的 Tauri shell 与发布包均未验证。
按用户调整后的口径，它们不阻塞本轮本地功能交付；若后续进行现场验收，需提供可用供应商配置、已配对实体设备及安装环境。
取消只承诺阻止后续授权及隔离迟到结果，不撤销已发生的 Effect，也不把 writer 接受字节当作用户已听到或机械动作完成。
