# macOS 开发环境 Agent 失败与延迟排查

## 调查对象与结论

对象：`bun run dev` 会话 `01a0db7e-6cc4-7012-a7e7-fe6ecf3edd9e`，请求为「帮我实现一个h5五子棋游戏」。
源码起点为 `eab7b8046`，已包含前一轮 Windows 修复。

此次失败发生在实际 macOS 文件/命令执行之前。主要问题是共享运行时将自动启用的任务账本误当作已关闭计划、工具发现批处理契约过严，以及 AgentExecution 没有消费子任务的持久暂停状态。不能归因于 Mac 文件系统较慢或通用 Agent 天生不适合开发。

只读查询的数据目录为 `~/Library/Application Support/NomiFun-dev-schema-b40e0155e59ea087`。
没有修改原会话、模型配置、工作目录或数据库状态。没有复制凭据和模型思考内容进入报告。

有一项与用户描述不同的事实：该会话保存的 Session-only preset 显示名为「通用」，其能力集合与通用模板一致，绑定模型为 `agnes-2.5-flash` / `openai_chat`，并非前次 Windows 排查的 StepFun。现有记录不能证明用户选错，也不能单凭这个差异断言前端选择器有缺陷。首页发送路径会核对并提交选中 preset；快速启动路径固定选择通用模板，需要实际入口信息才能进一步复现选择差异。

## 原始事件证据

| 事实 | 记录 |
| --- | --- |
| 父会话模型调用 | 9 步，第 9 步暂停 |
| 已收到的 usage 合计 | input 66,102 tokens；output 6,224 tokens；cache read 6,912 tokens |
| 工具结果 | 12 条，其中 11 条拒绝/错误；唯一实际执行的是委派 |
| 文件/命令执行 | 0 次文件写入，0 次命令执行，工作目录没有交付文件 |
| 上下文压缩 | 0 次；此次不能用压缩循环解释慢 |
| 父会话创建至暂停 | 129.31 秒 |
| 子会话 | `01a0db80-4fad-7e22-ba66-bc6ebbb24c4c` |
| 子会话暂停 | 创建后约 5.282 秒；2 个模型步骤、2 次工具搜索拒绝、0 次平台工具执行 |
| 子任务暂停至调度器判失败 | 1,800.03 秒，最后报 `agent_delivery_receipt_missing` |

父会话的具体链路：

1. 序号 20–21：第一次 `write_file` 生成正好用满 4,096 output tokens，整批因截断被丢弃，没有写入。
2. 序号 34–40：随后提出两个命令。调用数量触发 task ledger；新建的空计划被 `effect_gate` 判为 closed，整批未执行。
3. 序号 57–61：模型提出 plan + command，命令已经从下一轮工具表隐藏，整批被 unexposed 检查拒绝，计划也没建立。
4. 序号 80–105：两轮批量 ToolSearch 均被要求「只能单独一次调用」拒绝。模型搜索现成文件工具也无法消除计划造成的隐藏。
5. 序号 117–126：委派成功。但 parent 的自动账本已经启用，运行时没有采用普通委派的自动收尾路径，继续要求 plan / completion。
6. 序号 139–155：初次计划省略 schema 中可选的 explanation 被执行层拒绝；随后 completion 因没有计划再次被拒绝。
7. 序号 162–164：模型失败被统一转换为 `EXECUTION_MODEL_FAILURE` 暂停。原实现丢弃了更具体的错误类别，因此无法据此还原原次 HTTP 状态或供应商错误；不推断为限流或额度不足。

## 已实施的运行时修复

- **区分未建计划与关闭计划。** `revision=0` 且没有恢复要求时，自动账本只记录进度，不拦截已广告的合法文件/命令批次。显式关闭计划、用户追加输入、未知副作用和补丁恢复仍遵循原有约束。最终需求与完成证据核验保留。
- **初次计划的 explanation 真正可选。** 初次计划可由引擎补一个明确的初始说明；已有计划改范围和异常恢复仍要求解释，原始需求不可改写或丢弃。
- **允许纯 ToolSearch 批次。** 多个搜索可同批处理，每次仍只从冻结目录选最多五个已有 schema；搜索与执行/其他控制混合时整批拒绝。发现不授予新权限，不启动 Browser/Computer 等重型运行时。
- **检查后委派不再产生第二个完成责任方。** 对只有自动账本、没有显式计划/追加输入/成功写入或命令的父任务，成功委派可结束父回合。正在运行的进程、恢复要求和已有显式账本仍不能绕过。
- **将持久暂停传至调度器。** `AgentExecutionDelivery` 增加非终态 `paused_reason`，由已鉴权 Session owner 按精确 operation 查询。等待器看到该事实后立即返回明确、不可自动重放的未交付结果，交由现有调度器结算/清理；不再把已暂停子任务留到 30 分钟超时，也不将暂停伪装成完成回执。
- **保留模型失败类别。** 暂停码区分认证、无效请求、服务不可用、流中断和无效事件等，避免以后只剩一个泛化错误。仅保留类型码，不持久化原始供应商正文。
- **按宿主说明进程调用。** macOS/Linux 提供 `/bin/sh` + `args=["-c", "..."]` 例子及正确的 `ls` argv；Windows 保留 PowerShell 示例。
- **提前约束代码生成大小。** 明确输出额度包含推理与 JSON 转义，优先拆分 HTML/CSS/JavaScript 和完整的小调用。截断批次仍不执行，不拼接半段 JSON，也不自动重放旧副作用。

## macOS HTTP 客户端衔接问题

在当前 Mac 的系统代理配置下，应用集成测试首个模型请求即返回 provider unavailable，测试服务没有收到请求；仅给测试进程设置 `NO_PROXY=127.0.0.1,localhost,::1` 后，同一二进制的既有 Coding 用例通过。

源码发现 `EngineSessionHost` 使用独立的 `reqwest::Client::new()`，绕开项目已有的系统代理解析、本地排除项和连接上限。此路径也采用默认重定向行为，与 Broker 的单次请求边界不一致。

已改用共享 `nomifun_net::http_client_no_redirect()`；客户端构造失败经启动错误返回，不再隐式建立另一套传输策略。系统代理保持启用，没有修改用户代理设置。原会话使用远程供应商地址，这个本机服务问题不能直接充当原会话模型失败的归因。

## 通用与编程 preset 的全局设计判断

两者应共享同一执行协议、文件/进程实现、恢复和完成条件；preset 的区别是已授权能力与展示优先级。通用 preset 不应为了 coding 自动退化成「只能再委派给另一个 Agent」。

当前源码已经对较大工具目录进行按需展示，并把 workspace 的常用工具保留在首轮。构造相同任务的应用测试观察到 Coding 首轮 25 个工具、schema 22,236 bytes；通用首轮 21 个工具、schema 20,435 bytes。计数反映该夹具配置，不是所有用户配置的常量。它说明「通用工具更多，所以必然更慢」不是这次的充分解释。

另一个独立限制是模型元数据：原模型的 `context_limit` / `output_limit` 均未配置，运行时因此使用 32,768 / 4,096 的保守后备值。没有供应商确认依据时，本次不把模型别名映射为猜测的更大额度。较大真实代码生成应配合已确认的模型额度，并持续检验截断后的继续能力。

## 验证与边界

验证使用独立临时数据库和工作目录，模型输出采用可控的本地 HTTP/SSE provider，执行实际 Session → Broker → Runtime → Kernel → 文件/进程 owner 链路。新增场景覆盖两种 preset、截断丢弃、首次批量写入、含空格目录、macOS 原生 shell、交付与暂停传播。

| 检查 | 最终结果 |
| --- | --- |
| `cargo test -p nomifun-agent-runtime --lib` | 118 通过；覆盖首次批量写入、发现批处理、委派收尾，以及已有错误/恢复/完成证据约束 |
| `cargo test -p nomifun-agent-execution --lib attempt_runner::tests` | 18 通过；暂停不等待 30 分钟且不可当作成功或自动重放 |
| `cargo test -p nomifun-ai-agent --lib --features browser-use,computer-use unified_runtime::tests` | 12 通过；失败类型保留、资源清理和终态投影 |
| `cargo test -p nomifun-app --test native_coding_reliability --features browser-use,computer-use -- --nocapture` | 5 个测试通过；Mac 用例内部各执行一次 Coding 和通用 preset，共包含 6 个应用场景 |
| `cargo test -p nomifun-app --test native_execution_recovery --features browser-use,computer-use` | 3 通过；暂停/继续、崩溃接管及隔离待核对状态，没有重复写入 |
| `cargo test -p nomi-process-runtime --test process_contract macos_` | 4 通过；中文路径/argv/env/cwd、shell 退出状态、Seatbelt 写入范围和 TMPDIR 保护 |
| `cargo test -p nomifun-net --lib macos_managed_proxy_probe_reads_system_configuration` | 1 通过；既有受管进程探测能读取本机代理配置 |
| `bun run check:process-runtime-boundary`、`git diff --check` | 通过 |

上述最终应用验证没有设置额外 `NO_PROXY`，保留本机原有系统代理。首次失败的测试保留为诊断：原生客户端未采用项目代理策略；修复并重跑后通过。新 Mac 截断场景最初错误地要求被丢弃的调用 ID 出现在模型提示中，已改为检查 canonical discard 事件及磁盘无副作用，未放宽运行时的丢弃规则。适配层库测试还暴露两个旧夹具遗漏 `last_model_step` 初始化，已一并补齐。

本机验证输出保存在 `/tmp/nomifun-macos-*.log`，测试未向原用户数据库写入内容。它们不是长期可靠性评测数据集。

这些测试验证产品机制，不是原供应商生成完整可玩五子棋的真实模型验收。现有 StepFun live runner 没有可用测试凭据，且也不等同于原会话的 Agnes 路由。本次不发布成功率百分比，不声称已达到行业交付标准。

本轮没有修改 renderer，不需要桌面 UI 边界检查。未提交、推送或手动重启用户的开发进程。重新运行 `bun run dev` 后使用新会话验证；旧回合的恢复继续遵循 build digest、checkpoint、权限和副作用核对，不编辑数据库强行恢复。
