# nomi 驱动第三方 agent CLI(终端/子进程)设计

- 日期:2026-08-17
- 状态:**设计稿,未实施**。P0(引擎收敛到 nomi)已完成;本文是 P1 的方向性设计,由主人根据实际情况决定是否触发。
- 涉及仓库:`nomifun-desktop`(全部实现)
- 分支基线:`refactor/collapse-engines-to-nomi`(P0 收尾)
- 交付物(若触发实施):
  1. `docs/superpowers/specs/2026-08-17-terminal-driven-harnesses-design.md` — 本文
  2. TDD 实施计划(writing-plans 阶段产出)
  3. 代码 + 随码同行文档(`STATUS.md`、架构文档、`CHANGELOG.md`、`ui-api-contract-version.txt` 递增)

## 0. 背景与目标

P0 删掉了 ACP 以及 openclaw-gateway / nanobot / remote 四套适配层(净删 57k 行),只留原生 `nomi` 执行器。删的理由不是"第三方 agent 没价值",而是**每加一个第三方 agent 就要养一套协议适配代码**,兼容成本与测试成本压在每一次迭代上。

用户仍然想用 claude code / codex / gemini / grok / opencode / glm / kimi。目标是:**让 nomi agent 去驱动这些 CLI,而不是让 nomifun 去适配它们的协议**。nomi 已经会用工具了 —— 把"启动一个 CLI、给它下指令、等它干完、把结果读回来"变成 nomi 的一组工具,而不是平台的一个引擎。

**这不是把 ACP 换个名字重做一遍。** 区别是权责归属:ACP 时代平台承担"把第三方 agent 的会话/工具调用/审批语义翻译成 nomifun 语义"的责任,任何一端变更都要改适配层;这里平台只负责"起进程、喂 stdin、读 stdout、报告退出",语义理解交给 nomi 自己的推理。前者是 N 个协议实现,后者是 1 个进程管理。

### 已确认的方向性决策

| 决策点 | 结论 | 备选与否决理由 |
|---|---|---|
| 会话形态 | **不新增 `AgentType`**。第三方 CLI 是 nomi 会话里的一组工具,不是一种引擎 | 否决新增 `AgentType::Harness` —— 那正是 P0 删掉的形状,会重新引入闭合枚举的全仓库级联(P0 期间一个变体的移除牵动 21 个 crate) |
| 传输层 | **结构化 stdio 优先,PTY 兜底**(见 §1、§2) | 否决"统一走 PTY 屏幕抓取" —— 调研结论直接否决了这条路,见 §1 |
| 适配面 | **3 个解析器,不是 7 个**(Claude 系 / Codex / Gemini,外加 Kimi、opencode 两个适配) | 否决"一套通用适配器" —— "一次集成通吃"是假的,见 §1.3 |
| 完成判定 | **只信终止事件**,不信空闲启发式 | 否决 700ms 静默推断作为主判据 —— 仓库自己的 `submit.rs:63` 就写明它不可靠 |
| 审批 | **一次性预决策**(启动时定策略),不做逐次拦截 v1 | 否决 v1 做双向审批拦截 —— 只有 Tier A 支持,做了也只覆盖 2/7 |
| 幂等 | **复用既有的 turn admission**;写入不可逆,失败即 park,绝不自动重试 | 否决重试 —— stdin 写入没有回滚,重试等于重复执行 |
| 隐藏终端 | **不隐藏**。会话对主人可见 | 否决隐藏 —— 见 §6 诚实性问题 |

## 1. 关键前提:调研结论推翻了原始前提

原始设想是"通过**终端**驱动":起一个 PTY,把提示词打进去,盯着屏幕输出判断它干完了没有。**调研否决了这个方向。**

### 1.1 七个 CLI 里有五个提供结构化非交互模式

| CLI | 一次性调用 | 输出格式 | 回合结束信号 | 工具调用可见 | 单 stdin 多回合 | resume | MCP 注入 |
|---|---|---|---|---|---|---|---|
| **claude** | `claude -p "…" --output-format stream-json --verbose` | JSONL | `result` 事件,每回合恰好一个 | 是(`tool_use`/`tool_result` 内容块) | **是**(`--input-format stream-json`) | `--resume`/`-c`/`--fork-session` | `--mcp-config`(文件或内联 JSON) |
| **codex** | `codex exec --json "…"` | JSONL | `turn.completed`/`turn.failed` | 是(`command_execution`/`file_change`/`mcp_tool_call`) | 否 | `codex exec resume`/`--last` | `-c mcp_servers.*` / `codex mcp add` |
| **gemini** | `gemini -p "…" -o stream-json` | JSONL | `result` 事件 | 是(**顶层** `tool_use`/`tool_result`) | 否 | `-r`/`--session-id`;ACP 走 `--acp` | `gemini mcp add` / settings.json,无内联 flag |
| **grok** | `grok -p "…" --output-format streaming-messages-json` | JSONL,**Anthropic Messages 线格式** | `result` 事件 | 推断同 claude(未观测) | 否 | `-r`/`-c`/`--fork-session` | `grok mcp add`,无内联 flag |
| **kimi** | `kimi --print -p "…" --output-format=stream-json` | JSONL,**OpenAI 形状** | 进程退出 + 退出码(`0`/`1`/`75` 可重试);`--wire` 下有 `TurnEnd` | 是(`tool_calls` / `{role:"tool"}`) | **是**(`--input-format=stream-json`) | `--mcp-config` / `--mcp-config-file` |
| **opencode** | `opencode serve` 后 `POST /api/session/{id}/prompt` | HTTP JSON + SSE | `session.idle` | 是 | 不适用(HTTP) | `/api/session/{id}` | 未验证 |
| **glm** | `claude -p …` + `ANTHROPIC_BASE_URL=https://api.z.ai/api/anthropic` | 同 claude | 同 claude | 同 claude | 同 claude | 同 claude | 同 claude |

**结论:结构化 stdout 完胜屏幕抓取,而且它本来就存在。** 原设想里最难的两件事 —— 判断回合结束、看见工具调用 —— 在结构化模式里是免费的,在 PTY 模式里一个靠 700ms 启发式、一个根本拿不到。

### 1.2 因此 PTY 只是兜底,不是主路径

PTY 路径要付的代价,全部是仓库现状可核实的:

- **没有终端模拟器。** `nomifun_common::ansi` 只有 161 行,是个 ANSI 转义剥离器 —— 无 alt-screen(`?1049`)、无光标定位、无滚动区、无重绘合并。对一个全屏 TUI 做 `strip_ansi`,得到的是**每一帧重绘拼接起来的字符串**,不是屏幕内容。要变成"屏幕"得引入真正的 vt100 状态机,那是一个独立的大工程。
- **完成判定不可靠。** `submit.rs:20` 的 `IDLE_SETTLE_WINDOW = 700ms` 自带注释说明它是启发式;`SettleReason::Idle` 的文档写着 "never a definitive 'the agent declared done'"。
- **拿不到结构化工具调用。** ACP 免费给的东西,PTY 给不了。
- **写入不可逆。** `submit.rs` 的两段式写入(bracketed paste body,`TERMINAL_SUBMIT_DELAY=120ms`,再单独写 CR)是为真实 bug 修的:CR 与 paste-end 同批写会被 TUI 的 paste-burst 检测吞掉,提示词留在输入框里不执行。

PTY 仍然要留,因为有两个结构化模式覆盖不到的场景:交互式登录(`grok login --device-code`)、以及主人想亲手接管时。但它不承担自动化回合的完成判定。

### 1.3 "一次集成通吃"是假的 —— 但也没有七套那么糟

三个解析器,不是七个:

- **Claude 线格式**:`claude`、`grok`、`glm`(同一个 parser)
- **Codex**:自有事件词汇表(`turn.completed`)
- **Gemini**:形状像 Claude 但**不兼容** —— `tool_use` 在顶层、无 `subtype`、`message` 是平的。**不要共用 parser。**
- 外加 `kimi`(OpenAI 形状)、`opencode`(HTTP+SSE)两个适配

注入机制同样分三路,这一点仓库里已经实现过了(`enhance.rs`):claude 用 `--mcp-config`、codex 用 `-c mcp_servers.*`、gemini 只能写 system-defaults 文件。`AgentCli::supports_lifecycle_hooks` 的注释已经如实记录:gemini 没有启动期注入 hook 的机制,所以拿不到结构化 turn-end。

## 2. 已有的可复用资产(均在当前分支核实)

`nomifun-terminal` 10,237 行,不是从零开始:

**nomi 已经有 11 个终端工具**(`caps_terminal_ext.rs`):`nomi_create_terminal`、`nomi_list_terminals`、`nomi_terminal_get`、`nomi_terminal_send`(带 `wait` + `timeout_secs`,回执 `settle_reason` + `output_tail`)、`nomi_terminal_write_input`、`nomi_terminal_read_output`、`nomi_terminal_kill`、`nomi_terminal_delete`、`nomi_terminal_resize`、`nomi_terminal_relaunch`、`nomi_terminal_update`。

**companion 的系统提示已经在教 agent 驱动终端**(`companion.rs:188-196`),含 `preset: shell|claude|codex|gemini`。

**presets 已存在**(`ui/.../terminal/launchPresets.ts`):shell / claude / codex / gemini,各带 full-auto flag(`--dangerously-skip-permissions`、`--dangerously-bypass-approvals-and-sandbox`、`--yolo`)。P0 明确保留了这些资产。

**生产级的 写入→等完成→判定 循环已存在**(`auto_work_runner.rs::inject_and_wait_terminal`),而且它的诚实姿态正是本设计要继承的:
- `TerminalTurnEnd::AuthoritativeVerdict` vs `AmbiguousAfterSubmission` —— 后者"absorbing and must be parked for review, never retried"
- 无 lifecycle 时跑到硬超时,如实报 ambiguous,**不伪造 "done",也不做第二次 PTY 注入**

**其它可复用**:`TerminalDriver` trait(含 `write_input_exact_epoch` 的世代围栏,默认 fail-closed)、`submit.rs::encode_submit_chunks`、`enhance.rs::apply_enhancement`、`TerminalLifecycleServer`(`LifecycleKind::{TurnEnd,ToolUse,Notification,SessionStart}`)、`TerminalEventEmitter`。

## 3. 架构:一个新工具族,不是一个新引擎

```
nomi agent (唯一引擎)
  ├─ 既有工具:Bash / Read / Edit / Grep / Glob / 11 个 terminal 工具 / …
  └─ 新增工具族 nomi_harness_*  ← 本设计
        │
        ▼
  HarnessSession 服务(建议落在 nomifun-terminal,复用其进程与持久化)
        ├─ Tier A 驱动:长驻子进程 + 行读取 + 请求/响应关联
        ├─ Tier B 驱动:每回合 spawn + session-id 记账
        └─ Tier C 驱动:HTTP + SSE(opencode)
        │
        ▼
  三个解析器:ClaudeWire / CodexWire / GeminiWire(+ Kimi、opencode 适配)
```

**分层要点:**
- 传输是 `Stdio`(结构化,主路径)或 `Pty`(兜底,登录/接管)。`nomi_process_runtime` 的 `Transport::{Pipe, Pty}` 已经支持两者。
- 解析器只做"字节 → 事件",不理解 nomifun 语义。任何"这个工具调用意味着什么"的判断留给 nomi 的推理。这是与 ACP 的根本分界。
- 完成判定只接受终止事件(`result` / `turn.completed` / `session.idle` / 进程退出码)。拿不到就报 ambiguous。

### 分级(决定驱动实现,不是决定优先级)

- **Tier A — stdio 全双工**:`claude`、`kimi`(`--wire`)。一个长驻进程、多回合共用 stdin、可实时拦截工具/审批请求。驱动 = 长驻子进程 + 行读取器 + 请求响应关联器。**不需要任何空闲启发式。**
- **Tier B — 结构化、一次性、resume 续接**:`codex`、`gemini`、`grok`。终止事件干净,完成判定可靠,但每回合重新 spawn,审批必须预先决策。驱动 = 每回合 spawn + session-id 记账。若确需拦截,升级路径存在:`codex app-server`、`gemini --acp`、`grok agent stdio`。
- **Tier C — 带外 API**:`opencode`。完全不解析 stdout,直接驱动 HTTP + SSE。

`grok` 在**生命周期**上属 Tier B,但**共用 Tier A 的 parser** —— 它发的是 Claude 的信封。`glm` 不是独立集成,是换了环境变量的 `claude`。

## 4. 分期建议

**Phase 1 —— 只做 claude(Tier A)**。理由:唯一被实证确认支持单 stdin 多回合的主流 CLI,终止事件明确,MCP 可内联注入,而且它的 parser 同时覆盖 grok 和 glm。做完一个 Tier A 驱动,Tier B 是它的退化形式。

**Phase 2 —— codex + gemini(Tier B)**,两个独立 parser。

**Phase 3 —— grok / glm(复用 Claude parser,只改 launcher 与 auth)、kimi、opencode**。

不建议一次做七个:三个解析器 × 三种注入机制 × 两种传输,组合面太大,而 Phase 1 就能验证整个架构假设。

## 5. 明确不做(v1)

- **不做终端模拟器**。不引入 vt100 状态机。PTY 兜底路径只做 `strip_ansi` 的 tail,并如实标注它不是屏幕内容。
- **不做逐次审批拦截**。只有 Tier A 支持,v1 用启动期一次性策略(各 CLI 的 full-auto flag 已在 presets 里)。
- **不做 token/成本归因**。`result.total_cost_usd` 等字段存在但未在完成回合中观测过;先不展示,不如实就不写。
- **不做自动重试**。stdin 写入不可逆。
- **不隐藏终端**(见 §6)。

## 6. 诚实性问题:必须先改的一句提示词

`companion.rs:195` 现在向 agent 承诺:

> 主人在终端页能实时看到你的输入与执行,放心大胆地用。

原始设想里的"**隐藏**终端"与这句话直接冲突。二者只能留一个。

**本设计选择保留可见性,放弃隐藏。** 理由不是实现难度:一个能在主人机器上以 full-auto 权限跑任意命令的子进程,如果主人看不见它在干什么,那是把审计面主动关掉。P0 之后 nomifun 只有一个引擎,`session_mode` 默认 `yolo`、全类别自动批准 —— 在这个安全姿态下,可见性是唯一剩下的兜底。若将来确实要做隐藏模式,那句提示词必须同步改掉,且需要独立的审计决策,不能顺带实现。

## 7. 待验证 spike(实施前置)

调研在无凭证环境下完成,以下必须在有凭证的机器上跑通才能进实施:

**未验证清单(如实,不藏):**
- **七个 CLI 没有任何一个跑通过完整的已认证回合。** claude 撞 402 配额、codex API 超时、grok 未登录、gemini/opencode/kimi 从未对活模型运行。**所有成功路径的信封都是结构推断,不是观测结果。**
- claude 的 `can_use_tool` 审批往返:控制通道已确认活着(`initialize` 探针成功),但从未观测到真实的 `can_use_tool` 提示,也没发过裁决。
- claude 在 headless `-p` 下 hook 是否触发:未确认(模型调用先失败了)。
- codex 的 `turn.completed` 从未在运行时观测到,取自 SDK 类型声明。
- gemini 的 `result` 事件与全部字段名:读自 bundle 源码,非运行时。
- **opencode 的 `run` 模式整体未验证**;只核实了生成的 SDK 客户端的端点与事件名清单。
- grok 在 `streaming-messages-json` 下的工具调用块形状:从 Claude 兼容性推断,未观测。
- glm / kimi 的 Anthropic 兼容 base URL 来自第三方(claude-code-router 的 provider preset),**不是** Z.ai / Moonshot 官方文档。
- `/v1/messages/count_tokens` 与 `cache_control` 在 Z.ai / Moonshot 路径上是否支持:无人验证。假定不支持并降级。
- **所有文档 URL 都未读取**(调研环境 HTTP 全部被阻断)。引用的 URL 是待核指针,不是已读来源。

**Spike 清单:**
1. 对 claude 跑通一个**已认证的完整回合**,抓下真实的 `system/init` + `assistant` + `result` 三段,核对字段。
2. 用 `--input-format stream-json` 连喂两个输入,确认得到两个 `result` 且 `session_id` 相同。
3. 触发一次真实的 `can_use_tool`,发一次裁决,确认 `{behavior:"allow"|"deny"}` 的往返。
4. 确认 `--mcp-config` 内联 JSON 在 Windows PowerShell 下的可用性(仓库已有前例:`tauri.updater.conf.json` 走文件而非内联,因为 PS 5.1 会破坏内联 JSON)。
5. codex / gemini 各跑通一个已认证回合,确认终止事件的真实形状。

## 8. 陷阱(反直觉项,逐条一行)

- **grok-build 方向陷阱**:它的 `api_backend = "messages"` 是让 `grok` 做 Anthropic API 的**客户端**,不是 xAI 提供 Anthropic 兼容**服务端**。**绝不要**把 `ANTHROPIC_BASE_URL` 指向 xAI。(此条来自子代理,未独立核实;而且反过来读才是直觉答案)
- **`@xai-official/grok-win32-x64` dist-tag 错位**:其 `latest` 是 `0.1.220`,而 launcher 钉的是 `1.0.4` —— 版本要从 launcher 的 `optionalDependencies` 解析,不能查平台包的 dist-tag。
- **`claude --permission-mode` 的 help 没列出 `default`**,但 `default` 是实际运行时值(已观测)—— 别把 help 列表当穷举。
- **`claude` 的 `result.subtype:"success"` 可以带 `is_error:true`**(在一个 402 上观测到)—— 判定要看 `is_error` + `terminal_reason`,绝不能只看 subtype。
- **`codex` 的 `{"type":"error"}` 不一定致命** —— 观测到 `"Reconnecting... 2/5"` 是以 `error` 事件到达的。权威是 `turn.failed` + 退出码。
- **`codex -a/--ask-for-approval` 在根命令上有、在 `codex exec` 上没有** —— exec 要用 `-c approval_policy=…`。
- **`gemini` 的 `stream-json` 像 Claude 但不兼容** —— `tool_use` 在顶层、无 `subtype`、`message` 是平的。不要共用 parser。
- **`grok` 发 Claude 的信封但会留空字段** —— 观测到 `session_id: ""`、`model: "unknown"`、以及一个 `errors[]` 数组。放宽 `init`/`result` 的严格校验;全零的 `usage` 视为未知,不是免费。
- **`kimi` 的 flag 抄了 Claude(`--print`、`--input-format`、`--output-format stream-json`)但载荷是 OpenAI 形状** —— flag 兼容不等于 schema 兼容。
- **名字被抢注**:npm 的 `kimi-cli`(一个前端工具包)和裸 `grok-cli`(一个 claude-code 包装器)与 Moonshot / xAI 无关。官方是 PyPI `kimi-cli` / npm `@xai-official/grok`。
- **官方 CLI 的名字猜不出来**:xAI 的是 `grok-build`(二进制 `grok`),不是 `grok-cli`/`grok-code`。调研第一轮就因为猜名字得出了"xAI 没有官方 CLI"的假阴性 —— 要按 publisher scope / maintainer 邮箱枚举,不要猜仓库名。
- **`grok` 的平台二进制是 brotli 压缩的**(`grok.exe.br`)—— 若自行 vendor 而不走 launcher 的 postinstall,需要解压。

## 附:与 P0 删掉的东西的边界

P0 删了协议适配层,**没有**删掉本设计要用的东西。以下资产 P0 明确保留:

- `nomifun-terminal` 全部(含 claude/codex/gemini 三个 PTY preset 与 `enhance.rs` 的三种注入机制)
- 11 个 nomi terminal 工具与 `caps_terminal_ext.rs`
- `WriteSurface::Terminal`(P0 从 `TerminalAcp` 改名而来,保留 `WriteMode::Direct` 授权 —— 删掉它会让每个终端的 knowledge_write 被拒)
- `browser_stdio.rs` 与 loopback browser 桥(其 `BrowserSurface::Acp` 变体名字是历史遗留,与 ACP 协议无关,不在任何线格式/DB/UI 上)
- `process_registry.rs` / `boot_process_reaper.rs`(提供 `BootTerminalProofProvider` 依赖的开机孤儿回收证明)
- vendor logo 资产
- `factory_reset.rs` 的 `codex-acp-home` 条目 —— **冻结,零改动**。两个注册表逐元素比对持久化计划,删掉它会复现一个已知的 bricking 回归。
