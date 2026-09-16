# CAR 目标架构与 Port 合同

> 2026-09-14 校正：本合同按 CAR-D-019～021 和用户后续要求解释。
> 当前产品事实 owner 是 NomiCoreSessionOwner / ConversationService，不进行第二套
> Session 存储迁移；Engine 自己拥有任务循环、规划与上下文策略。共享 Runtime 仅
> 是生命周期托管，不是强制所有 Engine 使用的通用智能循环。早期隔离交付说明
> 不是当前完成状态；源码接线与未验证/未实施项见 STATUS 和
> [ENGINE-READINESS-2026-09-14.zh.md](ENGINE-READINESS-2026-09-14.zh.md)。

## 1. 总体结构

```text
Desktop / Web / Remote / Automation
              │
              ▼
NomiCoreSessionOwner / ConversationService
  ├─ 读取 immutable Snapshot
  ├─ 校验 AgentBinding / principal / active set
  ├─ 从已保存的 Agent Revision 解析 exact Engine Build
  └─ 经通用 Registry/Factory 创建绑定该 Build 的 Engine
          │
          ├─ ContextAssembler
          ├─ CodingToolPlan
          ├─ AgentModelPort ───────► nomifun-chat-model-broker
          ├─ CapabilityInvoker ────► nomifun-agent-kernel
          ├─ WorkspacePort ────────► nomifun-file / workspace owner
          ├─ ProcessPort ──────────► nomi-process-runtime
          ├─ VcsPort ──────────────► NomiFun VCS owner
          └─ Journal/EventSink ────► 原 Conversation owner
```

Runtime 在 NomiFun 进程内运行。多个 Engine Build 可以同时存在并服务不同
AgentSession。它可以请求 owner 创建命令子进程，但不创建 Codex Runtime 进程，
也不加载外部 app-server。

## 2. Engine Catalog 与 crate 边界

### 2.1 Agent Execution Engine Registry

平台需要一个轻量 Engine Catalog/Resolver：

- 注册 immutable Engine Family/Build；
- 维护指向 immutable Build 的 `stable`/`canary` channel alias；
- 按用户选择解析 exact Build；
- 为新 Session 解析不可变 `EngineBinding`；Fork 只继承父会话 exact binding；
- 不修改既有 Session Binding；
- 不提供自动 fallback。

Engine Catalog 只描述执行引擎，不管理 Model Route、Plugin Runtime 或业务能力。

### 2.2 早期隔离交付：`nomifun-coding-engine`（历史边界）

源分支最初交付低侵入、未接生产组合根的独立 crate；以下描述该历史边界，
不是当前产品仍未接线的结论。当前通用注册和生产宿主见 2.3～2.4：

- `CodingEngineCatalog`：管理 Coding family 内的 immutable Build 和 channel alias；
- `EngineBinding`：固定 `agent_session_id`、`runtime_binding_id`、exact Build digest、
  profile 和 `resolved_snapshot_ref`；
- `CodingEngineSession`：one-active-turn、cancel、dispose；
- `CodingModelPort`：直接复用 `ChatModelRequest`/`ChatModelEvent`；
- `CodingToolPlan`/`CodingToolInvoker`：携带 schema digest、canonical
  capability/action/resource/effect；
- Coding turn loop：text/reasoning/Tool Call/Tool Result continuation。
- `KernelCodingToolInvoker`：将 Tool Call 投影到已编译 Snapshot、active set 和
  `KernelRegistry`，不拥有 handler；
- 标准 Coding Tool exposure：Inspect/Edit/Execute/Full 只是显式 ToolPlan 筛选；
- `ManagedCodingProcessOwner`：复用 `nomi-process-runtime` 的 bounded output、
  stdin、PTY、timeout 和 process-tree cleanup；
- AGENTS.md、Context、Compaction、Checkpoint/Resume 的 bounded 合同。

该隔离 Catalog 不承担平台级异构 Engine Registry。当前产品采用
RuntimeEngineCatalog / RuntimeEngineHost，由本地主重构分支的生产 owner 接线，
同时容纳 Nomi、Coding 和源码注册实现；不是等待另一个远程工作进程接入。

### 2.3 共享生命周期与宿主端口，不共享强制执行策略

当前共享入口为 `nomifun-ai-agent::engine_sdk`、`nomifun-engine-core` 和 app 的
公共 Engine host facade，不要求新增名为 nomifun-agent-runtime 的 crate。

- RegisteredAgentRuntime 是已有产品调用者的实现扩展点。
- 可选 HostedAgentRuntime 为 EngineSessionDriver 提供 single-flight、取消等待、
  输出代号隔离、清理后持久终态和退出等待；它不实现模型任务循环。
- EngineSessionHost / EngineTurnJournal / EngineModelPort / EngineToolHost 等提供
  已准入的 Session、模型、工具、资源和历史事实。源码扩展也必须经这些 owner
  或等价的已准入平台接口执行，不直接访问凭据、数据库或原生文件来绕过权限。
- model step 调度、工具批次选择、规划、上下文窗口取舍、压缩/历史解释由各个
  Engine 自己实现。共享原语可以复用，但不强制继承 Coding 的实现或私有事件。

Agent 是配置/业务身份，Engine 是可复用执行策略，Runtime 是该实现的生命周期
句柄。不要再建立一套用户可见的 Runtime 身份或把 Engine 降格为通用循环的 prompt。

### 2.4 官方 Coding Engine 与独立 Engine

`nomifun-coding-engine` 拥有自己的 stream-driven loop、计划/需求/完成账目、
工具调度、AGENTS 范围刷新、版本观察、上下文预算/压缩和历史策略；
`nomifun-ai-agent::coding_runtime` 负责接入共享生命周期。File、Process、VCS、
模型与 Session 事实仍归平台，不是 Coding 的私有 owner。

Nomi 保留自己的通用执行循环，通过同一 Registry 实现接口接入，不强制改用
Coding 的规划器或压缩器。社区实现也可采用完全不同的循环。当前
`nomifun-app/examples/evidence_engine` 用研究问题/只读观察/证据综合的独立策略
消费公共宿主，不调用 Coding loop；它是源码参考，不是第三个官方预置。

官方默认注册 Nomi 和 Coding。社区只在源码组合入口注册描述符、工厂和准入
策略，重新编译打包后可由 Agent 工作台选择；组装冻结后拒绝注册，无运行期
安装、挂载、卸载或热换 Engine 的接口。源代码存在不等于这些链路已经运行验收。

### 2.5 现有 NomiFun owner

| 事实/能力 | Owner |
|---|---|
| AgentPreset/Revision/Snapshot/Binding | 现有 Agent Control Plane / Compiler；由 Nomi-core 投影准入 |
| 产品 Session / accepted turn / journal | NomiCoreSessionOwner、ConversationService 及原持久化 owner |
| Capability allowlist、schema、resource、handler | `nomifun-agent-kernel` |
| File read/search/write/patch/delete/snapshot | `nomifun-file` + File owner |
| Process/PTY/stdin/output/tree cleanup | `nomi-process-runtime` + Process owner |
| VCS status/diff/stage/commit/push | NomiFun VCS owner |
| Model route/provider/credential/retry/failover | `nomifun-chat-model-broker` |
| MCP materialization/connection | NomiFun MCP/Connector owner |
| Skill body/instructions/resources | NomiFun Skill Catalog |
| Plugin/MiniApp contribution/lifecycle/data | 各自 owning domain |

## 3. Port 设计原则

### 3.1 不复制现有 ChatModel schema

Runtime 可以提供一个窄的 `AgentModelPort`，但请求应包装或直接复用现有
`ChatModelRequest`、`ChatModelInput`、`ChatModelEvent`、`ChatCausality` 和
`CredentialLease` 语义，不另造一套长期并行的 Provider DTO。

建议形状：

```rust
#[async_trait]
pub trait AgentModelPort: Send + Sync {
    async fn open_stream(
        &self,
        request: nomifun_chat_model_broker::ChatModelRequest,
        cancellation: CancellationToken,
    ) -> Result<AgentModelStream, AgentModelError>;
}
```

`CancellationToken` 只存在于进程内调用，不进入序列化请求。

### 3.2 Tool Port

Tool Loop 不直接依赖具体 handler：

```rust
#[async_trait]
pub trait CapabilityInvoker: Send + Sync {
    async fn invoke(
        &self,
        snapshot: &CompiledSnapshot,
        active: &ActiveCapabilitySetSnapshot,
        request: CapabilityInvocationRequest,
        cancellation: CancellationToken,
    ) -> Result<StrictJsonValue, RuntimeToolError>;
}
```

最终实现由 `KernelRegistry`/Agent Platform 提供。每次调用都验证：

```text
Session + Snapshot + active generation
model-facing tool mapping
capability/action/schema
typed resources
principal/owner
effect class
operation/idempotency identity
```

### 3.3 Context Port

ContextAssembler 只接收：

- immutable Snapshot/RuntimeProfile；
- SessionEvent history reader；
- Workspace resource；
- AGENTS.md reader；
- Skill context；
- Tool plan；
- bounded tool results；
- token budget。

不能接收 root DB、Plugin instance、Credential store、全局 Catalog 或 Codex rollout。

### 3.4 Event Port

Runtime 通过 `SessionEventSink` 发送规范化语义事件和 transient stream：

- durable：turn started/completed/failed/cancelled、tool summary、final message、
  compaction summary；
- transient：text/reasoning delta、tool argument delta、progress；
- bounded partial：中断时最多一份可重建 partial。

Runtime private event ID 不得成为产品操作 ID。

### 3.5 当前取消能力边界

Coding 模型端口已使用 Broker 的可取消入口；只代表本地 attempt 的取消，不
承诺 Provider 服务端停止计算。工具等待被取消不等于工具效果回滚：已准入任务
由共享 EngineTaskGroup / EngineEffectScope 与具体 owner 持有、结算和清理。

HostedAgentRuntime 在 driver cleanup 成功之后记录同一终态，再发布产品 finish。
未知效果、缺失进程退出凭据或清理失败不能被一次 kill 请求抹除。跨启动丢失
进程树证明的情况仍隔离；平台端口并非所有外部协议的通用物理停止保证。

这些属于当前源码实现和必须保持的合同，端到端取消/清理验收仍未完成；不能
把源码中有 cancellation 参数或作用域类型当作实际退出证明。

## 4. 依赖图约束

允许：

```text
Agent Revision → Session owner → Registry → selected Engine
Engine → shared lifecycle primitives + admitted platform ports
Engine model strategy → ChatModelBroker contract
Engine tool strategy → Kernel invocation contract → File/Process/VCS owners
Engine event codec → admitted journal port → existing Conversation owner
```

禁止：

```text
Agent Runtime → root SQLite
Agent Runtime → Codex API/Auth/ModelProvider
Agent Runtime → app-server protocol
Agent Runtime → concrete Plugin/Browser/Computer instance
Engine strategy → concrete Conversation repository / NomiAgentManager
```

这里限制的是策略层旁路。生产宿主适配器本来就负责桥接当前 Conversation owner，
不能以此禁止宿主引用 ConversationService，或为了满足旧图额外建立 Session DB。

## 5. 一个 Engine Binding/Runtime Session 的生命周期

```text
在 Agent 工作台保存 Engine Family/Channel 或 exact Build 到 Revision
  → Resolver 解析 exact Engine Build
  → 将 Engine Build 写入该 Agent 新 Session 的 immutable Binding
  → load Snapshot
  → validate binding and Runtime feature
  → create actor
  → assemble context/tool plan
  → model step
  → execute zero or more Tool Calls
  → continue model step
  → complete / cancel / fail
  → dispose actor and transient resources
```

同一个 `RuntimeSessionActor` 不得：

- 中途换 Snapshot；
- 中途换 Engine Family 或 Engine Build；
- 静默更换模型 route；
- 从 Catalog 追加能力；
- 将 Tool 绕过 Kernel；
- 在失败后自动重放 uncertain external effect。
