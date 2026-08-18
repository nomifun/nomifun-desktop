# NomiFun 核心对话与 Coding 稳定性排查交接（2026-08-13）

> 状态：**未提交的 WIP 检查点**。已完成全面审查、首轮真实 StepFun Coding、多个 P0/P1 修复和大量定向测试；尚未完成修复后二次真实 E2E、整仓全量回归、若干持久化/前端交互残余项。接手者不得把本工作树直接视为已发布版本。

## 0. 给接手 Agent 的一句话

不要只修截图中的 `knowledge_search not advertised`。用户明确要求全面解决 NomiFun 的核心对话与工作任务稳定性：模型流、工具执行、Windows Shell、MCP、文件缓存、图片/知识工具面、后端终态、前端断线恢复、队列/确认交互都在范围内。

当前工作树没有已知半写语法，冻结态联合 `cargo check` 已通过；但端口 `18787` 上运行的仍是**修改前编译的旧 `nomifun-web.exe`**。下一步必须先审查差异、跑模块回归，再停止旧后端、重建并用隔离数据做第二轮真实 StepFun E2E。

## 1. 用户目标与范围

用户反馈：NomiFun 无法稳定 Coding，常在使用 2–3 分钟后卡死，或一触发工具就异常终止。用户要求：

1. 全面阅读 Agent 运行代码，而非只看单个报错。
2. 自己设计真实 Coding 工程任务，监督模型、流式协议、工具、持久化和 UI 全链路。
3. 使用 `StepFun Step Plan` 供应商和 `step-3.7-flash` 模型做本地隔离测试。
4. 发现并实际修复核心问题，再做功能和故障注入验证。
5. 后续又明确扩展：不能局限于截图，要全面排查核心对话、工作任务问题。

原截图包含三条连续故障：

- `NOMIFUN_INTERNAL_ERROR`：图片产物要求 verified receipt，但没有提交 receipt。
- `USER_LLM_PROVIDER_NETWORK_ERROR`：OpenAI-compatible stream 在 `finish_reason` 前结束。
- `USER_LLM_PROVIDER_GATEWAY_ERROR`：模型发出 `knowledge_search` 工具进度，但本请求没有广告该工具。

还出现图片生成模型未配置和知识回写模型不可用提示。

## 2. 安全、仓库与 Git 硬边界

- 工作目录：`C:\Users\Developer\code\nomifun\nomifun-desktop`
- 当前分支：`main`，跟踪 `origin/main`
- 分叉/冻结基线：`fae5658c22a7bb803088f664d4cc91c0a6d39383`
- 当前修改全部未提交、未暂存；不要 reset、checkout 或覆盖任何文件。
- 仓库绝对禁止 `.github/workflows/*.yml` / `*.yaml`。冻结时该目录只有 `README.md`。
- Git 提交必须是人类身份；不得出现 Codex/OpenAI/AI/bot 作者、提交者或 attribution trailer。未获用户要求前不要提交。
- 用户在原始请求中提供了 API Key。**本交接文件故意不记录、不复述该 Key。** 不要把 Key 放入源码、补丁、命令行、环境变量、测试输出或日志。
- 隔离数据目录含 UI 加密保存的凭据，应把整个目录视为敏感材料。完成验收前保留；最终清理时只能在核对绝对路径后删除精确目录。
- `cargo fmt --all -- --check` 在本 Windows 仓库会命中 `os error 206`；按本文列出的 package 集合格式化。

## 3. 当前实时运行环境（接手时必须重新核对）

冻结于 2026-08-13 时：

- 后端：`127.0.0.1:18787`
  - PID `29884`
  - 命令：`target\debug\nomifun-web.exe --port 18787 --host 127.0.0.1 --data-dir C:\Users\Developer\AppData\Local\Temp\nomifun-stability-019ffa15\data --api-only --insecure-no-auth`
  - **这是修改前编译的旧二进制，不代表当前源码行为。**
- 前端 Vite：`127.0.0.1:5173`
  - Vite Node PID `4008`
  - 上层 Bun PID `26336`
- 隔离运行根：`C:\Users\Developer\AppData\Local\Temp\nomifun-stability-019ffa15`
  - 冻结时约 57 个文件、7.3 MiB。
- 隔离 Coding 工作区：`C:\tmp\nomifun-stability-work-019ffa15`
  - 冻结时约 172 个文件、23.7 MiB。
- Provider ID：`019ffa27-53d8-7d48-a790-0bf3a12ce3e5`
- Conversation ID：`019ffa2a-798e-7232-9d3d-9d35658e8105`
- 浏览器页面：`http://127.0.0.1:5173/#/conversation/019ffa2a-798e-7232-9d3d-9d35658e8105`

后端使用 `--insecure-no-auth`，只允许保持在 loopback。不要改成 `0.0.0.0`。

核对进程：

```powershell
Get-NetTCPConnection -State Listen | Where-Object LocalPort -in 5173,18787
Get-CimInstance Win32_Process | Where-Object {
  $_.CommandLine -like '*nomifun-stability-019ffa15*' -or
  $_.CommandLine -like '*--port 18787*'
} | Select-Object ProcessId,ParentProcessId,Name,CommandLine
```

## 4. 首轮真实 StepFun 测试证据

### 4.1 Provider 和协议

- Provider preset：`StepFun Step Plan`
- Base URL：`https://api.stepfun.com/step_plan/v1`
- Model：`step-3.7-flash`
- UI 健康检查成功，约 960 ms。
- 官方文档：
  - [Step Plan Reasoning API](https://platform.stepfun.com/docs/zh/step-plan/integrations/reasoning-api)
  - [Chat Completion API](https://platform.stepfun.com/docs/zh/api-reference/chat/chat-completion-create)

两次低 token 原始流探测均 HTTP 200：

- 文本流同时出现 `reasoning` / `reasoning_content`；终态 `finish_reason=length`，随后 `choices: []` usage-only chunk 和 `[DONE]`。
- 工具流返回完整工具 id/name/arguments，终态 `finish_reason=tool_calls`，随后 usage-only chunk 和 `[DONE]`。

结论：当前正常 StepFun SSE 形态可被解析；截图的 EOF 更像偶发上游/代理截断，而不是 StepFun 永远不发终态。

### 4.2 Coding 工程任务

首个 fixture 位于 `C:\tmp\nomifun-stability-work-019ffa15`，任务是实现 `src/duration.ts` 并跑测试/typecheck。

观察结果：

- 模型先成功执行 Glob/Read。
- 第二个模型 pass 推理约 92 秒并达到 `max_tokens=8192`，`finish_reason=length`；引擎自动续轮，未发生传输断线。
- 最终代码实现成功，`npm test` 显示 23 pass / 0 fail。
- typecheck 因 fixture 自身声明 `types: ["bun-types"]` 但包配置不完整而失败，不应归因于 NomiFun 产品代码。
- 对话累计约 190 条持久消息并开始反复尝试，最终通过 UI Stop 取消；约 2.5 秒内后端恢复 idle、`active_turn_id` 清空、可再次发送。取消路径本次实测正常。

### 4.3 实测暴露的产品问题

1. Windows PowerShell 5.1 不支持 `&&`。模型执行 `cd C:\tmp\... && bun --version` 时失败。
2. 非交互 Windows 命令原先仍走 ConPTY，模型只看到 PTY 控制码，stdout/stderr 不清晰。
3. Bash schema 和 exec_command schema 不一致，模型直觉传 `timeout`/`yield_time_ms` 时会撞 oneOf 校验。
4. Write/Edit 后 Read cache 把新版本错误地标为“模型已经见过”，返回 `File unchanged...`，导致模型持续怀疑写入是否成功。
5. 真实任务长推理会自然达到 `length`；这不是网络断线，但弱工具反馈会使自动续轮放大成循环。

### 4.4 WebSocket 故障注入

任务运行时曾停止 Vite 进程树约 12 秒，再以同端口启动：

- 后端 turn 保持运行。
- 浏览器恢复后 transcript 和 running 状态可重新 hydration。
- 这次后端没有恰好在断线窗口内 terminal，因此没有真实命中“终态丢失”最小复现；该路径已通过静态追踪和确定性前端测试覆盖，但仍需要修复后二次浏览器 E2E。

## 5. 已定位的根因地图

### P0：2–3 分钟卡住 / 流错误

- Provider HTTP client 固定 connect 30s、per-read idle 120s，和用户的 2–3 分钟现象高度吻合。
- Agent 自身 1.2s idle tick 只发 preparing/activity，不终止整个 turn；SSE 心跳/滴流可持续重置 read timeout。
- 原 `http_client` build 失败会退回无超时 `Client::new()`。
- `ProviderError::Http` 原先不被 `is_retryable` 覆盖，零内容的 timeout/reset 也不重试。
- clean EOF 缺 `finish_reason` 被包装为 Connection，再误映射为 Network/Base URL 建议。
- partial stream 不能盲重放，否则可能重复文本或工具副作用。

### P0：工具一调用就挂

- MCP `tools/call` 原先没有通用 deadline。stdio mutex roundtrip、SSE `rx.await`、streamable HTTP 都可无限等待。
- SSE listener 退出后原先不 drain pending sender，调用者永久等待。
- generic `Tool` trait 仍没有全局统一 deadline；MCP 已专项修复，但其他自定义工具、approval `rx.await`、并发 `join_all` 仍有残余风险。

### P0：截图中的图片 + knowledge 工具协议冲突

- 中文图片意图检测用单字子串 `画`/`图`，`画布项目助手合并需求`、canvas、图表源码、UI 图标等 Coding 请求会误入生图路由。
- 挂载 KB 时 system/首轮 prelude 强制模型先调用 `knowledge_search`，但 strict image route 只广告 `image_gen` 或空工具。
- strict image tool 交付失败后原引擎继续第二个 provider pass，同时 tools 已被清空；模型继续遵从 KB 提示发 `knowledge_search`，触发 `not advertised`。
- ExplicitExternal 图片路由没有 durable external artifact bridge，却仍可打开 verified receipt gate，生成无法满足的内部错误。

### P0：前端断线后永远 running

- WebSocket 静默阈值 75s，无事件 replay。
- POST accepted 的 authoritative poll 在收到 `turn.started` 后因 lifecycle generation 变化失效。
- 若后端在断线期间完成，finish/terminal 丢失；重连只 reload transcript，不重新查询 authoritative runtime，UI 可永久 running，重试按钮又被 busy 隐藏。
- Nomi、ACP、Basic Runtime 三条路径都存在同构问题。

### P0/P1：异常终止后的 runtime 复用

- 异常 unwind 可能使 `TurnTeardownFence` 长期 pending；旧 runtime 仍被 registry 复用，下一轮卡住。
- exact cleanup 不能伪造成功，但后继回合也不能无限等。

### P1：Windows Shell 与 Read cache

- 非交互命令错误使用 PTY，控制码污染且 stdout/stderr 合并。
- 工具说明没有明确 PowerShell 5.1 不支持 `&&`/`||`，也没有强调进程已在 session workspace。
- File cache 把“用于写冲突校验的最新版本”和“已经展示给模型的版本”混为一谈。

### 仍未实现的已知风险

- Conversation DB finalize、generation bind、release 等多处无限重试，需要 durable `Finalizing/Quarantined` 设计，不能简单超时后释放 authority。
- relay `broadcast(128)` 和同一消费循环中的 SQLite await 可能触发 `Lagged` 并杀 turn。
- permission confirm HTTP 没有 deadline；失败只 console，按钮可永久处理中。
- `/messages` POST/command queue 没有 deadline，一个悬挂请求可永久锁队列。
- busy 且草稿非空时 Stop 会被 steer/send 替代，用户不易取消。
- Nomi message buffer / processed cron ID 无界增长。
- generic tool execution/approval 仍缺统一 deadline；并发 `join_all` 会让已完成工具被一个挂起 peer 拖住。
- WebSocket 在 socket open 时就广播 reconnected，早于首个有效 inbound frame；新 resync 有重试可缓冲，但 transport 健康语义仍不精确。

## 6. 当前工作树中的已完成修复

### 6.1 Provider / streaming

主要文件：

- `crates/agent/nomi-providers/src/lib.rs`
- `crates/agent/nomi-providers/src/retry.rs`
- `crates/agent/nomi-providers/src/openai.rs`
- `crates/agent/nomi-providers/src/anthropic_shared.rs`
- `crates/agent/nomi-providers/src/{anthropic,bedrock,gemini,vertex}.rs`
- `crates/backend/nomifun-ai-agent/src/protocol/send_error.rs`

完成项：

- 新增 typed `ProviderError::StreamTruncated`。
- Http connect/timeout/body/decode/request 错误纳入 retryability。
- HTTP client build 失败显式返回，不再退回无 timeout client。
- 仅 retryable `FailedEmpty` 自动重试；`FailedPartial` 继续 fail-closed，避免重复副作用。
- OpenAI/Anthropic/Gemini/Bedrock 缺终态的 clean EOF 改为 typed truncation。
- send_error 将 typed truncation 映射为 gateway/retry，而不是误导用户修改 Base URL。
- 增加 StepFun reasoning alias、usage-only、`length`、`[DONE]` 和 transport reset fixtures。

### 6.2 MCP deadline / transport cleanup

主要文件：

- `crates/agent/nomi-config/src/config.rs`
- `crates/agent/nomi-mcp/src/manager.rs`
- `crates/agent/nomi-mcp/src/transport/{mod,sse,stdio,streamable_http}.rs`
- `crates/agent/nomi-mcp/src/tool_proxy.rs`
- `crates/agent/nomi-cli/src/main.rs`

完成项：

- `McpServerConfig.request_timeout_secs: Option<u64>`。
- 默认每请求 90s，可配置 1–600s。
- initialize/list/call/resource 请求进入 manager deadline。
- timeout 调用 transport `abort_request`；stdio 退役整个旧 child/pipe，防止迟到响应串到下一请求。
- SSE pending 使用可在 Drop 中同步移除的 correlation map guard。
- SSE listener 退出和 close 时 clear pending，使 waiter 立即收到 channel closed。
- SSE/streamable HTTP 使用 bounded client（connect 10s、read 120s）。

注意：该模块最初由中途额度失败的子任务落地，现已通过联合编译和一个 silent transport 定向测试，但接手者仍要进行完整 `nomi-mcp` 测试和真实无响应 stdio/SSE/streamable HTTP integration。

### 6.3 图片、KB 工具面、artifact terminal

主要文件：

- `crates/backend/nomifun-ai-agent/src/factory/nomi.rs`
- `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs`
- `crates/agent/nomi-agent/src/engine/mod.rs`
- `crates/agent/nomi-agent/src/engine/set_config_tests.rs`

完成项：

- 排除 canvas/Mermaid/流程图源码/UI icon/SVG diagram 等 code-native visual；真实海报请求仍可走 native image。
- ExplicitExternal 且没有 durable bridge 时由 host 在 provider 前确定性说明，不开 receipt gate、不跑 Browser。
- strict artifact delivery failure 后立即 EndTurn，禁止 tools=[] 的第二 provider pass。
- strict request 增加通用 request-scoped tool authority：本次 `request.tools` 是唯一工具调用权威。
- image-only/empty strict route 抑制 KB one-shot prelude 和 autoRAG，且不消费 prelude。
- factory 的 KB system capability 由 owner、sink、mount、allowed_tools 的真实交集决定，不再硬编码 `has_search_tool=true`。
- knowledge_search/read 在 bootstrap 前加入 allow list；registry register 任一失败即 build error，不再打印虚假的 Registered。
- “模型未调用必需图片工具/工具无图片”映射为用户可操作的 provider empty response；ledger/CAS/persistence integrity 仍保持 Internal。

### 6.4 Teardown fence / runtime quarantine

主要文件：`crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs`

完成项：

- `TurnTeardownFence` 后继等待上限 7s。
- 超时后标记 transport broken，返回结构化 stream-broken，不伪造 cleanup 成功。
- `TurnTerminationGuard::drop` 在异常 unwind 立即 mark broken，registry 不再复用死 runtime。

### 6.5 Windows Shell 与文件缓存

主要文件：

- `crates/agent/nomi-tools/src/{bash,exec_command,windows_shell}.rs`
- `crates/agent/nomi-tools/src/{file_cache,read,write,edit,lib}.rs`
- `crates/agent/nomi-tools/tests/{edit_write_cache_test,file_cache_test}.rs`
- `crates/agent/nomi-types/src/file_state.rs`

完成项：

- Windows 非交互 `tty=false` 改用 Pipe；只有显式 TTY 使用 ConPTY。
- Pipe 保持 `CREATE_NO_WINDOW`，同时恢复独立 STDOUT/STDERR。
- Bash 支持结构化 `workdir`，默认已在 session workspace；仍走 capability/canonicalization 校验。
- 提示明确 Windows PowerShell 5.1 不支持 `&&`/`||`。
- 模型可见输出加入跨 chunk ANSI/OSC/C0 sanitizer；底层 raw/交互 PTY 不改写。
- FileState 增加 `dedup_eligible`：只有真正通过 Read 展示的版本才可返回 unchanged stub；Write/Edit 后下一次 Read 必须返回新内容。
- cache key canonicalize/lexical normalize；Windows 大小写归一。

### 6.6 前端 authoritative lifecycle resync

主要文件：

- `ui/src/renderer/pages/conversation/platforms/reconcileConversationTurnAfterStreamTerminal.ts`
- `ui/src/renderer/pages/conversation/platforms/nomi/useNomiMessage.ts`
- `ui/src/renderer/pages/conversation/platforms/acp/useAcpMessage.ts`
- `ui/src/renderer/pages/conversation/platforms/useAuthoritativeTurnLifecycle.ts`
- `ui/src/renderer/pages/conversation/platforms/BasicRuntimeSendBox.tsx`
- 相邻 lifecycle/structure 测试。

完成项：

- 共享 authoritative runtime 协调器，退避 `0ms → 120ms → 400ms → 1.2s → 3s → 8s → 16s`，单次 GET 3s deadline。
- BackendRequestError、timeout、unknown 均在循环内重试，不从 void Promise 泄露 unhandled rejection。
- Nomi/ACP/Basic 接受 `turn.started` 后把 poll ownership 转给新 generation。
- 三条路径订阅 `ws.reconnected`：idle 清 busy/settle；processing 采用 exact `active_turn_id` 并继续 poll。
- 初始 hydration 只有得到完整 processing/idle authority 后才开放 queue。

## 7. 冻结态验证结果

### 7.1 当前联合检查（工作树冻结后由主 Agent 重跑）

```powershell
cargo fmt -p nomi-agent -p nomi-cli -p nomi-config -p nomi-mcp `
  -p nomi-providers -p nomi-tools -p nomi-types -p nomifun-ai-agent -- --check

cargo check -p nomi-agent -p nomi-mcp -p nomi-providers `
  -p nomi-tools -p nomifun-ai-agent
```

结果：通过。仅有与本任务无关/既有的 `nomifun-model-invoke` dead-code 和 `nomifun-terminal` unused variable warnings。

补跑：

- `cargo test -p nomi-tools --test edit_write_cache_test`：10 pass / 0 fail。
- `cargo test -p nomi-mcp a_no_response_transport_returns_a_bounded_tool_error_and_is_aborted`：1 pass / 0 fail。
- `git diff --check`：通过。

### 7.2 已通过的模块回归

- `nomi-providers --lib`：159/159。
- `provider_openai_test`：16/16。
- provider truncation/error classification、MaxTokens 自动续轮、未提交工具 preview 不发布：定向通过。
- UI platforms：161 pass / 0 fail / 625 expect。
- `bun run --cwd ui typecheck`：通过。
- 根目录 `bun run check`：通过。
- `nomi-tools file_cache_test`：15/15。
- Windows shell：8 个 lib tests，加 PowerShell parse error、独立 STDERR、无 ANSI、含空格 workdir、cwd/TTY 双流定向测试均通过。
- 图片/KB：code-native visual、不支持 external bridge、strict artifact failure、strict authority prefix、KB prelude suppression 定向测试均通过。
- teardown fence：bounded stuck wait、exact completion、armed drop、broken runtime replacement 四项通过。

### 7.3 尚未做的验收

- 未跑完整 `nomi-tools`、`nomi-agent`、`nomi-mcp`、`nomifun-ai-agent` 全套。
- 未跑整仓 Rust 全量。
- 未重建 `nomifun-web` 后做第二轮真实 StepFun Coding。
- 未做“后端恰在 WS gap terminal”的真实浏览器故障注入。
- 未做真实 silent stdio/SSE/streamable HTTP MCP integration。
- 未验证修复后 3–5 分钟持续 coding、多个工具、取消、重连、provider reset 的组合场景。
- 未清理隔离数据和工作区。

## 8. 当前 Git 修改清单

冻结时共 40 个 tracked 文件被修改，另加本交接文档；全部未提交：

```text
crates/agent/nomi-agent/src/engine/mod.rs
crates/agent/nomi-agent/src/engine/set_config_tests.rs
crates/agent/nomi-cli/src/main.rs
crates/agent/nomi-config/src/config.rs
crates/agent/nomi-mcp/src/manager.rs
crates/agent/nomi-mcp/src/tool_proxy.rs
crates/agent/nomi-mcp/src/transport/mod.rs
crates/agent/nomi-mcp/src/transport/sse.rs
crates/agent/nomi-mcp/src/transport/stdio.rs
crates/agent/nomi-mcp/src/transport/streamable_http.rs
crates/agent/nomi-providers/src/anthropic.rs
crates/agent/nomi-providers/src/anthropic_shared.rs
crates/agent/nomi-providers/src/bedrock.rs
crates/agent/nomi-providers/src/gemini.rs
crates/agent/nomi-providers/src/lib.rs
crates/agent/nomi-providers/src/openai.rs
crates/agent/nomi-providers/src/retry.rs
crates/agent/nomi-providers/src/vertex.rs
crates/agent/nomi-tools/src/bash.rs
crates/agent/nomi-tools/src/edit.rs
crates/agent/nomi-tools/src/exec_command.rs
crates/agent/nomi-tools/src/file_cache.rs
crates/agent/nomi-tools/src/lib.rs
crates/agent/nomi-tools/src/read.rs
crates/agent/nomi-tools/src/windows_shell.rs
crates/agent/nomi-tools/src/write.rs
crates/agent/nomi-tools/tests/edit_write_cache_test.rs
crates/agent/nomi-tools/tests/file_cache_test.rs
crates/agent/nomi-types/src/file_state.rs
crates/backend/nomifun-ai-agent/src/factory/nomi.rs
crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs
crates/backend/nomifun-ai-agent/src/protocol/send_error.rs
ui/src/renderer/pages/conversation/platforms/BasicRuntimeSendBox.tsx
ui/src/renderer/pages/conversation/platforms/acp/useAcpMessage.ts
ui/src/renderer/pages/conversation/platforms/authoritativeTurnLifecycle.structure.test.ts
ui/src/renderer/pages/conversation/platforms/nomi/useNomiMessage.lifecycle.test.ts
ui/src/renderer/pages/conversation/platforms/nomi/useNomiMessage.ts
ui/src/renderer/pages/conversation/platforms/reconcileConversationTurnAfterStreamTerminal.test.ts
ui/src/renderer/pages/conversation/platforms/reconcileConversationTurnAfterStreamTerminal.ts
ui/src/renderer/pages/conversation/platforms/useAuthoritativeTurnLifecycle.ts
```

接手时先执行：

```powershell
git status --short --branch
git diff --stat
git diff --check
Get-ChildItem .github\workflows -File | Where-Object Extension -in '.yml','.yaml'
```

## 9. 推荐接手顺序

### 阶段 A：冻结差异审查与完整模块回归

1. 阅读本文件和 `git diff`；不要从头重做，也不要把不同 checkout 的事实带进来。
2. 优先审查大 diff：
   - `manager/nomi/agent.rs` 同时含图片/KB 和 teardown 两个子任务的改动。
   - MCP 改动由中途失败的子任务最初落地，虽已编译和定向测试，但需特别审查 shutdown、late response、proxy、serde compatibility。
   - Windows Shell/Read cache 是最后冻结的改动，需跑完整 `nomi-tools`。
3. 依次运行：

```powershell
cargo test -p nomi-tools
cargo test -p nomi-mcp
cargo test -p nomi-providers
cargo test -p nomi-agent
cargo test -p nomifun-ai-agent
bun test --cwd ui src/renderer/pages/conversation/platforms
bun run --cwd ui typecheck
bun run check
```

若耗时太长，先逐包串行，不要并行多个 Cargo 进程争抢 target lock。

### 阶段 B：重建后第二轮真实 StepFun E2E

1. 精确停止 PID `29884` 对应的旧后端；不要杀其他 NomiFun 实例。
2. 重建 `nomifun-web`，用相同 loopback 端口和隔离 data dir 启动。
3. 保留现有 encrypted provider 配置，禁止输出/导出凭据。
4. 新建一个**无外部类型依赖**的干净小工程，避免再次被 `bun-types` fixture 噪声干扰。
5. 任务至少覆盖：Glob/Read → Write/Edit → 再 Read → Bash/exec_command → tests → 最终回答。
6. 验收：
   - Write 后第一次 Read 必须返回新内容，第二次相同 Read 才可 unchanged。
   - Windows 命令看到清晰 STDOUT/STDERR，无 PTY 控制码；模型不再反复 `cd ... &&`。
   - `step-3.7-flash` 可以完成一个终态，不陷入工具重试环。
   - Stop 始终能恢复 idle；新 turn 不复用 broken runtime。

### 阶段 C：故障注入

1. WS：让后端在前端断开窗口内完成，恢复后 2–20 秒内权威 settle、send/retry 恢复。
2. MCP：silent stdio、SSE listener EOF、streamable HTTP 不响应；确保在配置 deadline 内得到 ToolResult error，下一工具请求不消费迟到响应。
3. Provider：首事件前 reset 可安全有限重试；已产生 partial 内容后不重放，只发一个 terminal error。
4. 图片 + KB：
   - `画布项目助手合并需求` / canvas code / Mermaid 源码不得走生图。
   - 真正“生成海报图片”在无模型时 deterministic 提示；无 external bridge 时不得开 artifact receipt gate。
   - strict artifact 失败只发生一个 provider pass，不再发 hidden `knowledge_search`。

### 阶段 D：继续修残余核心问题

优先级建议：

1. 前端 `/messages` POST 与 permission confirm deadline、幂等 receipt/retry、可见错误和按钮恢复。
2. busy + draft 时 Stop 始终可见。
3. Nomi buffer/processed ID 有界化。
4. generic Tool/approval deadline 与并发完成结果及时发布。
5. relay Lagged 与 DB projection 解耦。
6. durable Finalizing/Quarantined 状态，替代 persistence 无限 retry；必须保留 exact receipt/cleanup authority，不能以超时伪造成功。

## 10. 最终清理与交付闸门

只有修复后二次 E2E 和必要回归完成后，才清理隔离数据：

```powershell
$dataRoot = 'C:\Users\Developer\AppData\Local\Temp\nomifun-stability-019ffa15'
$workRoot = 'C:\tmp\nomifun-stability-work-019ffa15'
Resolve-Path -LiteralPath $dataRoot
Resolve-Path -LiteralPath $workRoot
```

确认解析结果精确位于上述两个显式目录后，再停止其进程并使用 `Remove-Item -LiteralPath ... -Recurse -Force`。这是不可恢复删除；交付时应告诉用户已删除什么。不要递归删除 workspace 根、Temp 根、`C:\tmp` 根或任何变量未解析的路径。

最终至少通过：

- 相关 Rust 包完整测试。
- UI platform 测试、typecheck、根 `bun run check`。
- `git diff --check`。
- `.github/workflows` 下 YAML 数量为 0。
- 一次完整真实 StepFun Coding 成功终态。
- WS terminal-gap、silent MCP、provider reset、图片/KB 组合故障注入。
- 检查源码、文档、命令记录中没有 API Key。

## 11. 不要误判的事项

- 92 秒推理后 `finish_reason=length` 是模型达到 token 上限，不等于网络断线。
- 首轮 `duration.ts` 的 typecheck 红项来自测试 fixture 配置，产品测试 23/23 已通过。
- `artifact receipt` / CAS/hash ledger 是正确的安全边界，不要为了“成功”而放松验证。
- partial provider stream 不能直接自动重放；只有明确零内容、retryable 的请求才安全有限重试。
- teardown timeout 不能清除 cleanup debt 或伪造资源已释放；正确行为是 quarantine/broken + 后续 exact cleanup。
- 当前 Vite 可热更新 UI，但当前后端旧进程没有加载任何 Rust 新改动。

## 12. 可直接复制给新 Agent 的启动指令

```text
请接手 C:\Users\Developer\code\nomifun\nomifun-desktop 的 NomiFun 核心对话/Coding
稳定性任务。先完整阅读：
docs/handoffs/2026-08-13-nomifun-core-conversation-stability-wip.md

当前 main 基线是 fae5658c22a7bb803088f664d4cc91c0a6d39383，工作树有 40 个 tracked
修改和一份 untracked handoff，全部是上一 Agent 未提交的 WIP。不得 reset/checkout/覆盖，
不得创建 .github/workflows YAML，不得以 AI 身份提交。用户 API Key 不在 handoff 中；不要
要求把它写进代码/命令行。隔离 data dir 已有 UI 加密 provider 配置。

先执行 git status、git diff --check、定向 package fmt/check，并审查 manager/nomi/agent.rs、
MCP、Windows shell/read cache 的大 diff。然后按 handoff 第 9 节跑模块测试。注意 18787
当前是修复前旧 nomifun-web 二进制：完成静态/测试审查后精确停止旧 PID，重建，再用原隔离
data dir 和 step-3.7-flash 做第二轮真实 Coding 与 WS/MCP/provider/image+KB 故障注入。

不要只解决截图；继续覆盖未完成的 queue/permission deadline、Stop 可见性、buffer 有界、
generic tool deadline、relay Lagged 和 durable Finalizing/Quarantined。每项必须保留幂等、
verified artifact receipt、exact cleanup authority，不能为了“不卡”伪造成功或盲目重放副作用。
```

## 13. 2026-08-14 接续进展：确认卡生命周期与 Nomi post-process 上限

本节是后续接手时的最新增量；仍为**未提交 WIP**，未 reset、checkout、commit、amend 或
清理隔离目录。

### 13.1 已修复

1. 权限确认卡不再因 ACP watchdog timeout 永久停留：
   - pending-confirmation 去重和 remove 同时识别 `permission.content.call_id` 与
     `acp_permission.content.tool_call.tool_call_id`。
   - `turn.completed` 只清理该精确 `turn_id` 的 permission/acp_permission 卡，不会删除下一
     turn 的卡。
   - turn 完成后重新读取 durable confirmation list，清理没有 `turn_id` 的
     `confirmation:*` 恢复卡。
   - 重叠的 list 请求使用序列栅栏，旧 HTTP snapshot 不能晚到后复活已清理的卡。
2. Nomi 本地 post-process 的 `inFlight` 已限制为
   `MAX_NOMI_PENDING_POST_PROCESSES`：
   - 不淘汰正在运行的任务。
   - 超限请求保留在 bounded pending map，槽位释放后继续。
   - 只有明确因 in-flight 容量/重复 target 等待的请求会被槽位释放唤醒；普通处理失败不会
     被并发 peer completion 反复热重试。
3. `PermissionRouter::start()` 已做一次性启动保护，重复调用不再生成长期等待 receiver
   mutex 的第二个 task/Arc owner。
4. `insert_pending_for_test_with_generation` 已限制为 test build，并清理了本文件新增测试中的
   unnecessary `mut` warning。

### 13.2 新增/扩展回归

- `usePendingConfirmationsRecovery.test.ts`
  - 两种权限卡 call-id 去重与删除。
  - turn 完成只删除精确 owning turn，保留下一个 turn 和 turnless recovery card。
- `usePendingConfirmationsRecovery.reconnectResync.structure.test.ts`
  - reconnect、turn-completed recovery 与 stale snapshot fence。
- `nomiPostProcessState.test.ts`
  - `inFlight` 到达 128 后不增长、不淘汰运行任务，overflow 保留 pending。
  - 普通失败 pending 不会被误当成 released-slot waiter。
- `permission_router.rs`
  - 重复 `start()` 只保留一个 receiver owner。

### 13.3 2026-08-14 最新验证

- `cargo test -p nomifun-ai-agent`：通过；lib 904 pass / 3 ignored，其余 package
  integration/doc tests 通过（需要外部 JSON-RPC mock agent 的既有测试保持 ignored）。
- `bun test --cwd ui src/renderer/pages/conversation/Messages src/renderer/pages/conversation/platforms`：
  484 pass / 0 fail。
- `bun run --cwd ui typecheck`：通过。
- 根目录 `bun run check`：通过。
- `cargo fmt -p nomifun-ai-agent -- --check`：通过。
- `git diff --check`：通过。
- `.github/workflows` YAML 数量：0；目录内仍只有 `README.md`。

### 13.4 当前运行进程与下一步

- Vite：PID `4008`，端口 `5173`。
- backend：PID `36212`，端口 `18787`。
- backend 启动时间早于本节的 `PermissionRouter::start()` 幂等修复；若继续真实 ACP timeout
  验收，必须精确重建/重启该 backend，不能把当前进程当作最新 Rust 行为。
- 下一项建议继续阶段 D 第 1 项：审计并修复 permission confirm 与 `/messages` POST 的
  deadline、幂等 receipt/retry、可见错误与按钮恢复；不要删除现有隔离数据。
