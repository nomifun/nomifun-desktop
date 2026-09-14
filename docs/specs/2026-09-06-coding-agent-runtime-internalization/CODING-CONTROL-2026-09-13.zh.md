# Coding 控制循环与恢复接线（未验证）

分支固定为 `rf/agent-capability-platform-v2`。仅本地修改，未 push。
本轮没有运行构建、测试、模型任务或对照评测，历史通过结果不覆盖本轮代码。
本文件续接 `CODING-LOOP-2026-09-13.zh.md`，不代表 CAR 全部能力已经完成。

## 已实现的增量

- **状态化计划**：engine 内置 `update_plan` 控制工具，不虚构 Kernel capability。
  每轮最多 16 个唯一步骤、64 次修订、最多一个 in_progress；修改文件或执行命令前
  需要活动步骤。失败后置为 needs_replan，下一次副作用前必须更新计划并给出解释。
  串行批次失败时，后续调用返回未执行，不能继续盲跑。计划与效果调用不能混在同一
  批次。结束时有一次纠正机会，未关闭的计划不能被接受为正常完成。
  计划及失败状态持久化，当前计划独立保留于压缩之外。状态为模型报告，不是完成
  证明；blocked 可以正常交接。它是模型驱动、engine 执行约束的计划机制，**不是
  无需模型的通用自主规划算法，也不是自动验证任务成功的判定器**。
- **路径感知指令**：经已授权的 fs.read，在文件 read/write/delete、typed patch
  文件路径及进程启动 cwd 上读取根到目标目录的 AGENTS 层级。同层 override 优先。
  新规则、规则修改或删除先更新有界视图并暂停整批调用，让模型看到后重新提交。
  串行调用之间重新检查，避免前一调用修改指令后，后一调用沿用旧视图。读取失败或
  截断时不执行该批操作。指令读取使用唯一回合内 operation id，变更事件可回放。
  无 fs.read 授权时只报告不能加载，不绕过授权读取，也不从 process grant 推导文件授权。
- **交互进程**：`process.exec` 的规范 schema 增加 exec/start/poll/stdin/
  close_stdin/resize/cancel；官方 Coding 的 `exec_command` 暴露这组操作。
  使用平台组合根持有的 `CodingProcessScope`，经 Kernel/Wave2 后委托现有
  `ManagedCodingProcessOwner`/`nomi-process-runtime`。句柄绑定用户、Session、
  当前回合 correlation 和服务端 workspace；不能跨回合操作。
  最多 64 次启动，单进程输出保留 256KiB、最长 10 分钟、单次轮询最长 30 秒。
  支持 Pipe/PTY、增量输出游标和已清理终态缓存；running 不是成功退出，重复轮询
  不重复计入命令完成。正常结束前必须显式轮询到终态或取消剩余进程，避免声称已完成
  而命令仍在运行。结束/取消先关闭启动入口并取消进程，随后 join 已接纳的工具，
  最后持久化 `host_cleanup_proven` 再发布业务终态。无法证明 reaped 则保留隔离。
- **保守异常恢复**：平台新增 `RegisteredEngineRestartRecovery` 编译期扩展点，
  按 exact family/build/digest 分派；注册随应用组装关闭。公共 Conversation 不解析
  Coding 日志，不把它送进 Nomi 的 rewind 路径。Boot provider 先核对启动时冻结的
  user/Session/epoch/operation，再调用相应 engine 的恢复实现。
  Coding 恢复检查精确 build、连续事件日志和 accepted receipt；没有 process
  admission，或有同一代的持久化 cleanup witness 时，关闭未终结的历史为
  “重启中断”，随后由平台原有生命周期 CAS 解封会话。缺失的工具结果仍标为副作用
  未知；不回滚文件，不重放命令，不自动继续旧回合。
- **产品提示及身份**：工作台不再错误提示“不支持进程执行”。构建标识更新为
  `<app-version>-host2-coding-loop2`，新增模块纳入开发指纹。仍是 Agent revision
  选择 engine、Session 冻结 exact build；没有聊天页切换或运行时动态挂载。

## 仍需后续实现 / 明确保留的限制

1. **执行中进程崩溃恢复不是全覆盖**：有 process admission 但没有持久化清理证明
   的回合继续隔离。Job/watchdog 的父进程退出保护不能替代启动后的精确持久化树级
   终止证明。没有把“内存中没句柄”当作恢复证据。进一步需要跨启动的 containment
   身份/清理证据，以及明确的人工处置入口。
2. **不是指令文件的全仓库扫描器**：shell 文本中任意动态路径、递归目录删除的所有
   子目录、符号链接别名与外部并发修改不属于当前路径推导保证。模型仍须按规则检查
   实际操作路径。读取与执行之间不是文件系统事务，不能宣称消除 TOCTOU。
3. **不是跨回合后台服务**：进程随回合清理；长期资源、按需能力、Skills、MCP、
   MiniApp、附件、snapshot/push 的 Coding 接入不在此切片。
4. **不是旧构建迁移或自动 checkpoint 续跑**：旧 exact build 仍不自动切换。
   恢复的是可安全关闭的历史和继续发起新回合的能力，不是恢复活进程/继续执行旧命令。
5. **验收待做**：新增模型调用、内置控制工具、schema digest、自动指令读取与恢复
   行为改变了既有 fixture 假设。需另行调整/运行针对性测试、三平台进程检查和真实
   编码任务评测；本轮依用户要求没有执行。

## Codex 借鉴来源

继续阅读指定本地源码 `multi/codex` 中的 `tools/handlers/plan.rs` 与
`tools/handlers/unified_exec.rs`：借鉴内置控制工具与权限能力分离、启动返回句柄、
stdin/后续轮询和有界等待机制。结合此前 AGENTS/compaction 阅读，按 NomiFun
平台所有权重新实现；不引入 Codex 的私有 Session、凭证管理或 app-server。
不以“参考 Codex”推导实现成熟度、通过率或优于 Nomi 的结论。
