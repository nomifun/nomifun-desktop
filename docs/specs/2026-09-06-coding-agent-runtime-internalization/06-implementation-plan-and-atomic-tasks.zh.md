# CAR 实施计划与原子任务

## 1. 施工原则

CAR 不是把整个 Codex 仓库搬进 NomiFun，而是按可独立验收的切片逐步形成一条
新的 in-process Runtime 主链。

每个任务必须做到：

- 可以单独启动、验证和提交；
- 有明确的输入和唯一写集；
- 不需要读取其他任务的临时状态；
- 不修改上一阶段文档；
- 不依赖外部 Codex 进程；
- 失败时可以普通 revert 或停留在明确的 `blocked`；
- 完成后有可复查的行为测试和边界检查。

任务机器合同见 `TASK-MANIFEST.json`，本文件负责解释任务边界和依赖。

## 2. 依赖图

```text
CAR-00 Source / License / Boundary
   ↓
CAR-00A Multi-Engine Catalog / Binding / Gray Release
   ↓
CAR-01 Coding Engine Core
   ↓
CAR-02 ChatModelBroker Adapter
   ↓
CAR-03 Tool Loop / Capability Mapping
   ├──────────────→ CAR-04 Process / PTY / Cancellation
   ├──────────────→ CAR-05 Patch / File / VCS / Workspace
   └──────────────→ CAR-06 Context / AGENTS / Compaction / Resume
                              ↓
                         CAR-07 Session Cutover
                              ↓
                         CAR-08 Legacy Removal
                              ↓
                    ┌─────────┴─────────┐
                    ↓                   ↓
              CAR-09 Ecosystem       CAR-10 Release
              Consumer Integration   Validation / Cutover
```

`CAR-04`、`CAR-05`、`CAR-06` 可以在 CAR-03 完成后由不同 Worker 并行，但不得
同时修改同一个共享模块或根 Cargo 文件。`CAR-07`、`CAR-08`、`CAR-10` 是中央
集成任务，必须串行。

## 3. 原子任务卡

### CAR-00：源码、许可证和边界冻结

目标：只完成调研和实施输入冻结，不改生产代码。

输入：

- `../codex` commit `6af345407d9c2a568da9d01b6c4b81a9e61495c0`；
- 本目录 00～02 文档；
- NomiFun 当前 `nomifun-codex-runtime`、`nomifun-chat-model-broker`、
  `nomifun-agent-kernel`、`nomifun-agent-platform`。

允许写入：

```text
docs/specs/2026-09-06-coding-agent-runtime-internalization/
  CODEX-SOURCE-MANIFEST.json
  01-audit-and-source-baseline.zh.md
  02-target-architecture-and-port-contracts.zh.md
  STATUS.zh.md
  TASK-MANIFEST.json
```

禁止：

- 修改 `docs/specs/2026-08-28-agent-capability-platform-v2/`；
- 修改任何 Rust/Cargo/Schema/打包文件；
- 启动 `codex-app-server`；
- 提出 Voice、Realtime、Guardian 或 Codex provider/auth 的迁移；
- 把旧 Wrapper 改成“过渡实现”。

交付：

- 固定源 commit；
- 选择性 `copy/adapt/rewrite/exclude` 清单；
- owner/Port 替换矩阵；
- LICENSE/NOTICE 和传递依赖审计要求；
- 任务依赖与禁区。

验收：

- 所有选定源路径存在；
- JSON 可解析；
- 许可证入口可定位；
- 没有生产 `../codex` path dependency 的新增变更；
- `git diff --check` 通过。

### CAR-00A：多执行引擎、Build、Binding 与灰度合同

目标：建立多个 Agent Execution Engine 并存时的最小选择和绑定模型，不接入旧生产路由。

允许写入：

```text
crates/backend/nomifun-coding-engine/src/engine.rs
crates/backend/nomifun-coding-engine/src/lib.rs
```

交付：

- Engine Family/Build descriptor；
- Stable/Canary channel alias（Catalog 指向 immutable Build，不写入 Build 本身）；
- Exact Build 和 Channel selector；
- immutable EngineBinding；
- 同一 Catalog 中多 Build 并存；
- 既有 Session 不可被切换或自动 fallback 的不变量。

验收：

- stable 与 canary 可以指向不同 Build，也可以将同一已验证 Build 从 canary 提升为 stable；
- exact digest mismatch fail closed；
- channel 解析只返回对应 channel；
- 两个 Session 可以同时绑定不同 Build；
- Binding 创建后没有 mutate/silent fallback API。

### CAR-01：Coding Engine Core

目标：建立第二个专门 Coding Engine 的 in-process actor 和 Port。

允许写入：

```text
crates/backend/nomifun-coding-engine/**
```

交付：

- `CodingEngineSession`；
- `CodingModelPort`；
- `CodingToolInvoker`；
- `CodingEventSink`；
- turn admission；
- stream-driven model step；
- bounded Tool arguments/results；
- cancellation/dispose/panic cleanup；
- 无工具 plain-text turn；
- Coding Engine 只通过 EngineBinding 运行，不接旧 Runtime 路由。

禁止：

- 引入 Codex API/Auth/History/Rollout；
- 引入 app-server protocol；
- 直接访问 root SQLite 或具体 owner；
- 修改旧 `nomifun-codex-runtime`。

验收：

- fake model 可以完成一个普通 turn；
- actor 只允许一个 active turn；
- cancel/dispose 幂等；
- Tool argument/result 有界且 actor panic 不遗留 active-turn 状态；
- crate 不包含 provider-specific 依赖。

当前本地实现将 `CAR-00A` 与 `CAR-01` 作为一个未接生产主链的隔离 crate 交付。
未来是否物理拆分通用 `nomifun-agent-runtime`，由远程集成者根据真实复用边界决定；
不得为了目录命名复制第二份 actor、turn loop 或 Port。

当前本地同一交付还包含 `CAR-03` 的 Kernel Tool admission seam、`CAR-04` 的
`nomi-process-runtime` adapter，以及 `CAR-06` 的 bounded Context/AGENTS/
Compaction/Checkpoint contracts；这些切片仍需远程主工作进程接入 AgentSession
和 SessionEvent 后才算产品完成。

### CAR-02：ChatModelBroker Coding 适配

目标：让 Coding Engine 通过 NomiFun Broker 获取完整规范化模型事件。

允许写入：

```text
crates/backend/nomifun-coding-engine/src/model.rs
crates/backend/nomifun-chat-model-broker/src/**
```

必要的共享 Cargo 修改由中央集成者在本任务串行窗口完成；Worker 不自行修改
根 `Cargo.lock`。

交付：

- `AgentModelPort` 到 `ChatBrokerPort` 的 adapter；
- `ChatModelRequest`/`ChatModelEvent` 映射；
- reasoning、Tool Call delta/completed、Tool Result、usage、terminal event；
- cancellation propagation；
- `AgentCompaction` typed task 或等价 request kind；
- exact route/causality/credential 校验。

验收：

- text → Tool Call → Tool Result → next model step 可闭环；
- semantic output 前允许 Broker policy retry/failover；
- semantic output 或 Tool Effect 后不自动 failover；
- Runtime 不接触 credential value；
- 至少覆盖现有 Broker 的主要协议能力筛选。

### CAR-03：Tool Loop 与 Capability Mapping

目标：把模型 Tool Call 可靠地转换为 NomiFun Capability invocation。

允许写入：

```text
crates/backend/nomifun-coding-engine/src/tool.rs
crates/backend/nomifun-coding-engine/src/turn.rs
crates/backend/nomifun-coding-engine/src/events.rs
```

交付：

- bounded argument delta assembly；
- model-facing tool plan；
- canonical capability/action/resource mapping；
- Snapshot/active-set admission；
- Tool Result normalization；
- read-only parallel 和 effectful serial policy。

验收：

- Snapshot 外 Tool Call fail closed；
- schema/resource/owner mismatch 返回 typed failure；
- 多个 Tool Call 的结果顺序确定；
- tool completed 后同一用户 turn 继续下一次 model step；
- 不直接调用具体 handler。

### CAR-04：Process、PTY、stdin 与取消

目标：将 Codex Unified Exec 的成熟执行语义接入 NomiFun Process owner。

允许写入：

```text
crates/backend/nomifun-coding-engine/src/process.rs
crates/shared/nomi-process-runtime/**
crates/backend/nomifun-agent-domain-wave2/**
```

交付：

- argv/cwd/env 解析；
- bounded head/tail output；
- stdin/write/poll；
- PTY（平台支持时）；
- timeout/cooperative cancellation；
- descendant process cleanup；
- exit code/signal/truncated/duration。

禁止：

- Codex approval/network permission；
- 把命令执行包装成外部 Agent Runtime；
- 将 provider secret 注入 argv/env。

验收：

- Windows/Unix 代表性命令；
- 长输出截断；
- stdin；
- timeout 后 process tree 为零；
- cancel 不产生重复执行。

### CAR-05：Patch、File、VCS 与 Workspace

目标：保留 Patch/Coding 文件操作能力，同时只使用 NomiFun 文件和 VCS owner。

允许写入：

```text
crates/backend/nomifun-coding-engine/src/patch.rs
crates/backend/nomifun-coding-engine/src/workspace.rs
crates/backend/nomifun-coding-engine/src/vcs.rs
crates/backend/nomifun-file/**
NomiFun VCS owner 的专用模块和测试
```

交付：

- strict patch parsing；
- context/hunk/path validation；
- atomic file mutation；
- workspace typed binding；
- VCS status/diff/stage/commit；
- result/rollback/error normalization。

验收：

- invalid patch、context mismatch、duplicate hunk；
- workspace containment；
- patch 失败不产生半写入；
- diff 反映真实文件变化；
- commit 通过 canonical VCS owner；
- 不存在第二套 Codex file owner。

### CAR-06：Context、AGENTS.md、Compaction 与 Resume

目标：从 NomiFun SessionEvent 重建 Coding 模型上下文。

允许写入：

```text
crates/backend/nomifun-coding-engine/src/context.rs
crates/backend/nomifun-coding-engine/src/compaction.rs
crates/backend/nomifun-coding-engine/src/checkpoint.rs
crates/backend/nomifun-coding-engine/src/agents_md.rs
```

交付：

- SessionEvent → model history；
- Workspace/AGENTS.md precedence；
- bounded context；
- retained facts；
- NomiFun compaction task；
- disposable checkpoint；
- resume/steer/follow-up。

验收：

- 无 Codex rollout 时可以重建下一次模型输入；
- compaction 不改变 Session/Snapshot/route identity；
- checkpoint mismatch 可丢弃并重建；
- AGENTS.md containment 和大小边界；
- cancel/steer 不重新提交 uncertain effect。

### CAR-07：AgentSession 主链切换

目标：让产品 application service 直接创建 in-process Runtime actor。

允许写入：

```text
crates/backend/nomifun-agent-platform/**
crates/backend/nomifun-app/src/**
Cargo.toml
Cargo.lock（由中央集成者生成）
```

交付：

- `AgentSessionService` → Runtime actor；
- AgentBinding/Snapshot admission；
- Remote/Automation/Chat/Coding 使用同一 Session 主链；
- 删除文本专用 bridge 的限制；
- fake/live model 的平台测试。

禁止：

- 长期保留旧 Wrapper 与新 Runtime 双主链；
- 同一 Session 切换或同时调用两个 Engine；
- 修改一期设计文档；
- 把旧 `/api/presets` 作为兼容输入。

验收：

- `open → turn → tool → continuation → completed`；
- `cancel`、`dispose`、crash recovery；
- SessionEvent 与 UI projection 正确；
- no sidecar process；
- route/credential/capability 都来自 NomiFun。

### CAR-08：旧 Wrapper、Sidecar 和打包残留删除

目标：物理删除旧外部 Runtime 的生产可达路径。

允许写入：

```text
crates/backend/nomifun-codex-runtime/**
crates/backend/nomifun-agent-contracts/contracts/runtime/**
crates/backend/nomifun-agent-contracts/src/runtime.rs
crates/backend/nomifun-agent-contracts/src/event.rs
crates/backend/nomifun-app/src/bootstrap/runtime_artifact.rs
crates/backend/nomifun-app/src/router/remote_runtime.rs
scripts/desktop-build-*.sh
scripts/gate-agent-v2.mjs
scripts/release/**
scripts/validation/**
```

交付：

- 删除旧 crate 和路由；
- 删除 app-server executable packaging；
- 删除 hello/native_action/session_dispose schema；
- 删除 sidecar artifact/release lock 字段；
- 将旧测试迁移或删除；
- 生产 reachability scan。

验收：

```text
生产 Cargo graph 无 nomifun-codex-runtime
生产代码无 codex-app-server 启动
无 runtime/hello、native_action/start、session_dispose 路由
无 runtime_sidecar/sidecar_artifact 生产 schema
无 RuntimeStartTurnBrokerBridge
```

历史文档、Git 历史和本目录的审计记录不属于生产 reachability。

### CAR-09：平台生态与非 Agent Consumer

目标：验证 Plugin、MiniApp、MCP、Skill 的贡献仍是平台能力，而不是 Coding
Runtime 私有扩展。

允许写入：

```text
Capability Catalog / MCP / Skill consumer adapter 的专用模块和测试
```

交付：

- MCP Tool 先物化为 canonical Capability；
- Skill 只提供 instructions/workflow/resources；
- Plugin/MiniApp Active contribution 通过 Catalog；
- Agent 与至少一个非 Agent consumer 使用同一 contribution；
- non-Agent-only contribution 不进入 Agent ToolPlan。

验收：

- ContributionLock provenance；
- Agent picker 过滤；
- non-Agent operation lock；
- Plugin/MiniApp lifecycle 不被 Runtime 接管；
- 没有 Agent-only Catalog 或 Package-owned Preset。

该任务不阻塞 CAR-01～CAR-08 的 Coding 核心闭环，可在核心稳定后实施。

### CAR-10：三平台验证与发布切换

目标：在不改变历史阶段文档的前提下完成新 Runtime 的发布门禁。

允许写入：

```text
本目录 STATUS/TASK-MANIFEST
新阶段专用 validation/release scripts
必要的 packaging 配置
```

交付：

- Windows Desktop x64；
- macOS Desktop arm64；
- Linux Desktop x64；
- in-process Runtime build/package/install/launch；
- Coding E2E、cancel/crash/dispose、secret scan；
- 同一 RC bytes；
- 旧 Wrapper production reachability 为零。

验收：

- 每个平台真实 Host 验证；
- 不用旧 Sidecar 产物冒充结果；
- 失败项有明确 `not_run`/`blocked` 原因；
- Stable 只提升已经验证的同一 RC。

## 4. 任务并行规则

允许并行：

```text
CAR-04 Process
CAR-05 Patch/File/VCS
CAR-06 Context/Compaction
```

前提是 CAR-03 已完成，且不争用：

- 根 `Cargo.toml`；
- 根 `Cargo.lock`；
- AgentSession central composition；
- shared SessionEvent registry；
- 同一个 Runtime crate 文件。

必须串行：

```text
CAR-00 → CAR-00A → CAR-01 → CAR-02 → CAR-03
CAR-07 → CAR-08 → CAR-10
```

## 5. 任务回传格式

每个任务完成后只回传以下内容：

```text
task_id:
base_sha:
commit_sha:
changed_paths:
checks:
not_run_and_reason:
acceptance_result:
blockers:
central_paths_touched:
follow_up:
```

禁止回传 credential、API key、主机地址、完整模型响应或包含秘密的日志。
