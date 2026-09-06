# CAR Tool Loop 与 Coding Capability 方案

## 1. 目标

本文件定义模型 Tool Call 如何进入 NomiFun Capability 主链。Codex 的 Tool
Registry、Router 和 parallel dispatch 只提供行为参照；最终 Tool identity、
授权、资源和 owner 由 NomiFun 掌握。

## 2. Tool Loop

```text
读取 Session 固定的 exact EngineBinding
  → 校验 request Session/Snapshot 与 Binding 一致
  → 构造当前 Snapshot 的 ToolPlan
  → 发送模型请求
  → 接收 text/reasoning/tool-call delta
  → 有界拼接 Tool arguments
  → 完成 Tool Call
  → 解析 model-facing mapping
  → 校验 Session/Snapshot/active set
  → 校验 action/schema/resource/effect
  → 调用 Capability Kernel
  → 规范化 Tool Result
  → 写入 SessionEvent/Transient stream
  → 将 Tool Result 放回模型 history
  → 继续同一 turn 的下一次 model step
```

一个用户 turn 可以包含多次 model step，但不能把每次 Tool Call 错误地当成新的
用户 turn 或新的 Snapshot。Tool Loop 的完整生命周期始终运行在同一个 selected
Engine Build 下；Tool continuation、取消或模型错误都不能切换 Engine。

## 3. 双重身份模型

模型面名称可以尽量保持 Coding Agent 已验证的名称；内部 canonical identity
必须始终通过映射表解析。

| 模型面名称 | Canonical Capability | Action |
|---|---|---|
| `exec_command` | `process.exec` | `run` |
| `write_stdin` | `process.exec` | `write_stdin` |
| `apply_patch` | `fs.patch` | `apply` |
| `read_file` | `fs.read` | `read` |
| `search` | `fs.search` | `query` |
| `git_status` | `vcs.status` | `read` |
| `git_diff` | `vcs.diff` | `read` |
| `git_add` | `vcs.stage` | `stage` |
| `git_commit` | `vcs.commit` | `commit` |

这张表是设计示意。最终名称、schema 和 action 以 NomiFun canonical Capability
inventory 为准，不能由模型字符串直接调用 handler。

## 4. Tool Admission

每一次调用必须同时验证：

```text
agent_session_id
resolved_snapshot_ref
active_set_generation
model_tool_name
canonical capability_id
action_id
tool schema digest
typed resource binding
principal / owner
effect class
operation / idempotency identity
```

任一字段缺失、不匹配或无法比较时，返回 typed failure，不能尝试“最接近”的
Capability，也不能从全局 Catalog 临时补能力。

当前隔离 `CodingToolBinding` 已将 `schema_digest` 作为显式 admission 字段，并以
canonical JSON digest 校验模型 tool definition。远程 Kernel adapter 仍需把该
digest 与 Snapshot 中的 `RuntimeCapabilityExecutionContract.schema_digest` 对齐；
不能只相信模型收到的 JSON schema。

## 5. 三类 Effect

### 5.1 `read_only`

适用于读取文件、搜索、status、diff 和没有共享可变状态的观察操作。

- 可在满足 owner 条件时并行；
- 不建立通用 Effect receipt；
- 结果进入 bounded Tool Result 和必要的 Session 摘要。

### 5.2 `managed_effect`

适用于 NomiFun 能以事务、CAS、原子文件操作或明确 owner 生命周期管理的动作：

- `fs.patch`；
- workspace 文件写入；
- VCS stage/commit；
- 本地受管 process。

默认串行，结果由 owning domain 负责最终事实。

### 5.3 `external_uncertain_effect`

适用于远程命令、外部服务、网络发送或无法保证结果可知的动作：

- dispatch 前创建最小 idempotency/reservation；
- unknown 时不自动 retry；
- reconcile 由 owning domain 负责；
- Runtime 不建立全局 EffectCoordinator。

## 6. 并行规则

Codex parallel dispatch 只在以下条件成立时使用：

- 所有调用都是 `read_only`；
- 没有共享可变上下文；
- 没有相同 process handle、worktree、Computer target 或 owner lane；
- 每个 call 有独立 cancellation 和 result identity。

`managed_effect` 和 `external_uncertain_effect` 默认串行。即使只读调用并行完成，
写入模型 history 和语义事件的顺序也必须按模型返回的 `call_id` 顺序确定，不能
把调度时序泄漏给下一轮模型。

并行只读任务共享 per-turn cancellation token，但每个 owner 调用仍应派生自己的
子 token/operation identity；取消后不得等待一个不响应取消的 Tool 无限返回。当前
隔离 loop 已在等待层对 cancel fail-fast，底层 owner 的真实终止由后续 Kernel/
Process adapter 保证。

## 7. Process 能力

`process.exec` 复用 Codex Unified Exec 的成熟行为，但 owner 归 NomiFun：

- 明确 argv、cwd、env；
- 非秘密环境继承；
- bounded head/tail output；
- stdin/write/poll；
- PTY（平台支持时）；
- per-command timeout；
- cooperative cancellation；
- timeout 后 descendant tree hard kill；
- exit code/signal/truncated/duration；
- Windows、macOS、Linux 的平台差异测试。

禁止把 Codex sandbox、Guardian approval、network approval 或 Codex process registry
原样带入。安全边界由 Snapshot、typed workspace、NomiFun Process owner 和
FullAuto admission 提供。

## 8. Patch、File、Workspace、VCS

### 8.1 Patch

可以抽取 Codex patch parser 的严格解析和诊断：

- context mismatch；
- invalid hunk；
- duplicate hunk；
- path validation；
- bounded patch size；
- atomic failure。

真正的文件读取、写入、anchored containment、rollback 和 CAS 必须进入
`nomifun-file` owner，不能保留第二套 Codex file owner。

### 8.2 Workspace

Workspace 是 typed resource，不是任意字符串路径。Tool 调用和 ContextAssembler
都必须通过同一个 Workspace binding，不能分别解析两个 cwd。

### 8.3 VCS

`vcs.status`、`vcs.diff`、`vcs.stage`、`vcs.commit` 和 `vcs.push` 使用 NomiFun
VCS owner。Agent 可以请求 commit，但是否真正 commit 由 Snapshot action allowlist
和 NomiFun FullAuto 合同决定；不复制 Codex approval UI。

## 9. Skill、MCP、Plugin、MiniApp

这些能力不是 Coding Runtime 私有 Tool registry：

```text
Skill / MCP / Plugin / MiniApp Release
  → Platform Capability Catalog
  → AgentPreset explicit selection
  → Snapshot ContributionLock
  → ToolPlan
  → Capability Kernel
```

- Skill 只提供 instructions/workflow/resources；
- MCP Tool 必须先物化为 canonical Capability；
- Plugin/MiniApp 可以服务 Agent，也可以只服务 Gateway、UI、Automation 或其他系统；
- Coding Runtime 不加载 Plugin source，不管理 MiniApp lifecycle；
- non-Agent-only contribution 不出现在 Agent ToolPlan；
- 未发布 Candidate、Source 或测试 Host 不进入正式 Snapshot。

## 10. 后置能力

以下能力不阻塞 Coding P0：

- Code Mode；
- Node REPL；
- Web Search；
- 完整 Subagent workflow；
- 多 Agent graph；
- 语音/Realtime；
- 自动安装或自动发布 Plugin/MiniApp。

如果后续需要，必须创建新的 `CAR-*` 任务和真实消费者，不得把后置能力偷偷加入
当前 ToolPlan。
