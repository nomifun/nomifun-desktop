# 共享 MCP stdio 进程接入（2026-09-14，未验证）

任务：CAR-03 / CAR-05 / CAR-07。在本地 `rf/agent-capability-platform-v2` 修改源码，
未 commit/push；未构建、测试、实际启动 MCP 服务、调用模型或执行数据库迁移。

## 接入范围

canonical MCP owner 现在接受 `stdio`，并与 HTTP/SSE 复用 `McpSession` 的初始化、
完整目录分页、冻结工具 schema 核对、服务器消息处理和工具结果校验。产品目录物化及
运行时资源解析同时接受三个传输类型；不调用旧 Nomi MCP manager，也不绕过 Kernel。
初始化仍使用已实现的 `2025-03-26` 协议，不宣称支持 Codex 新版免 initialize 协议。

设置页 stdio 目录发现也已复用该 owner，删除旧的不受限 read_line/首个响应协议路径。
保留命令缺失/权限错误的安装提示，只有完整目录与清理均成功才返回成功结果。
旧 HTTP 设置页目录发现尚未迁移；SSE 已在上一切片完成迁移。

## 进程与权限

- 命令、argv、环境来自经授权的持久 MCP 服务器配置，不从模型工具输入解析。
  工具输入仅进入 stdin JSON-RPC。MCP connect/invoke 资源授权不转化为 Engine 的
  任意 `process.exec` 权限，不为模型开放新的命令拼接入口。
- 复用 `nomi-process-runtime::ChildProcessBuilder::spawn_managed`。Windows 沿用隐藏
  窗口和平台 Job，Unix 沿用进程组/平台 watchdog；不是只持有一个可复用 PID。
- 增加 builder 的显式 `env_clear`，不改变其他适配器默认行为。MCP 子进程只继承
  PATH、用户目录、系统/临时目录及必要本地化变量，叠加显式 server env；不自动继承
  应用 API token、带凭据的代理配置或 Node 注入变量。显式配置的环境仍由用户负责。
- Windows 环境键按大小写无关规则去重/覆盖。命令配置限制 256 个参数、128 个环境键、
  256KiB 总字符串；拒绝 NUL/非法环境键。按子进程 PATH 解析裸命令并固定路径，
  不把 argv 拼接成 shell 字符串，不自动切到某个 Agent workspace。
- 这是已授权本地 MCP 程序执行，不是 OS 沙箱。最小环境不能阻止该程序读取它本来就有
  权限访问的磁盘或发起网络请求，不能据此声明强隔离。

## 协议预算与关闭

换行分帧保持跨 chunk 的 UTF-8 与 CRLF；单帧最多 8MiB、全事务最多 32MiB/4096 帧。
stdout 严格作为 JSON-RPC，非法行、未完成帧 EOF 或不匹配 ID 会失败；stderr 丢弃，
不新建无界日志缓存/独立 reader task，也不将诊断文本暴露给模型。

整帧 stdin 写入持有互斥锁，请求同时读取响应，避免双向管道背压。服务端 ping/显式
不支持方法响应共用这条写入路径；不授予 sampling、elicitation 或 roots 等权限。
分页保持既有 32 页/1024 工具/8MiB 目录界限及全部页面去重、schema 比对。

命令解析/创建在 blocking task 执行，启动和协议阶段共用 owner 截止时间。启动前再核对
截止时间；若创建已进入 OS 调用则不声称能撤销。取消等待/启动超时后，晚到的受管进程
结果仍会进入平台清理责任链，不自动重新启动。

正常结束、协议失败或超时均先关闭 stdin，最多给 250ms EOF 退出宽限，然后调用受管
进程的全树 shutdown。关闭预算为 10 秒；即使主进程已退出也必须核对全树清理。
只有真实工具响应及清理成功才返回 owner 成功；清理失败覆盖较早结果，宿主保留未知
效果隔离。中断清理的责任由既有平台 relay 接续，但后台接续不等于当前调用已证明清理。

每次调用拥有一份临时 MCP 进程，不复用前次进程状态。清理证明也不代表已写文件、
远端请求或工具效果被回滚；stdio 进程仍可能在启动/初始化时产生外部效果。

## 引擎关系、参考与剩余事项

Nomi、Coding 和编译期社区 Engine 均消费相同 Session/Kernel MCP 工具宿主。
stdio 是工具传输，不是打包后安装或挂载 Engine；Engine 仍只允许源码集成并随应用注册。
当前 Coding 为 `host2-coding-loop36`，Nomi 为 `host20`，摘要纳入 stdio owner 和改动的
平台命令 builder；旧 exact Session 不自动升级或改绑。

参考本地 Codex `rmcp-client/src/bounded_stdio_transport.rs`、`stdio_server_launcher.rs`、
`local_stdio_transport.rs` 和 `utils.rs` 的有界帧、完整写入、环境白名单及进程放置边界。
进程树清理沿用 Nomifun 已有平台责任，不复制第二套跨平台 PID 清理实现。

新增代码未编译/执行验证。MCP resources/订阅、经授权的 server-initiated 生命周期、
持久 stdio 会话、额外远端进程 placement、新版协议、其他生态生命周期、跨启动证据及
人工隔离处置仍未完成，不能据此宣称 Engine 总体能力已完善或发布就绪。

## 2026-09-25 市场连接修复补充

当前实现对本文最初记录的环境和 stderr 策略作了两处收敛式修订：

- `env_clear` 仍不继承 API Token、Node 注入变量或其他无关父进程环境，但会通过
  `nomifun-net` 的统一代理策略注入显式父进程代理变量，或在其缺失时注入系统代理；
  server 自己配置的代理仍优先，并继续使用失效 loopback 代理防护。这使桌面进程启动的
  `npx`/`uvx` 首次依赖下载与应用内 HTTP 请求采用同一网络边界。
- stderr 不再直接丢弃。独立 reader 持续排空 pipe 以避免子进程背压，但只在最初 64KiB
  范围内设置固定失败分类位；原始字节不会保留、写日志或返回 API。reader 在受管进程
  cleanup 时有界 join，超时即 abort，不形成无界后台任务。

设置页手动连接检测还会识别 package runner：普通握手继续使用 30 秒预算，`npx`、
`bunx`、`uvx` 等首次下载与握手共用 120 秒预算。该差异只属于可用性检测；canonical
owner 仍使用调用方给定的单一 deadline，未增加自动重试、后台安装或第二套进程所有权。
