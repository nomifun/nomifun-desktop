# CAR 目标架构与 Port 合同

## 1. 总体结构

```text
Desktop / Web / Remote / Automation
              │
              ▼
AgentSession Application Service
  ├─ 读取 immutable Snapshot
  ├─ 校验 AgentBinding / principal / active set
  ├─ 解析用户选择的 exact Agent Execution Engine Build
  └─ 创建一个绑定该 Build 的 RuntimeSessionActor
          │
          ├─ ContextAssembler
          ├─ CodingToolPlan
          ├─ AgentModelPort ───────► nomifun-chat-model-broker
          ├─ CapabilityInvoker ────► nomifun-agent-kernel
          ├─ WorkspacePort ────────► nomifun-file / workspace owner
          ├─ ProcessPort ──────────► nomi-process-runtime
          ├─ VcsPort ──────────────► NomiFun VCS owner
          └─ SessionEventSink ─────► nomifun-agent-session
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
- 为新 Session/Fork 生成不可变 `EngineBinding`；
- 不修改既有 Session Binding；
- 不提供自动 fallback。

Engine Catalog 只描述执行引擎，不管理 Model Route、Plugin Runtime 或业务能力。

### 2.2 当前隔离实现：`nomifun-coding-engine`

当前电脑先交付一个低侵入、未接生产组合根的独立 crate：

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

这个 Catalog 只是 Coding Engine family 的隔离 Build Catalog，不承担最终平台级
异构 Engine Registry。远程主工作进程在 `CAR-07` 增加统一 Registry/Factory，
同时容纳 Legacy Nomi Engine 和 Coding Engine，并将新 Session/Fork 的选择解析为
exact `EngineBinding`。

### 2.3 远程集成后的通用 `nomifun-agent-runtime`

只拥有通用 Agent 执行机制：

- `RuntimeSessionActor`；
- turn admission；
- model step；
- Tool Call delta assembly；
- Tool continuation；
- cancellation/timeout/backpressure；
- Context window accounting；
- Compaction orchestration；
- Resume/checkpoint cache；
- runtime event normalization。

不得拥有：

- provider-specific client；
- root SQLite；
- Capability catalog；
- domain repository；
- Plugin/MiniApp lifecycle；
- credential value。

当前隔离 crate 验证通过后，远程集成者可以按实际复用边界决定是否物理拆出通用
crate；在拆分发生前，不得为了匹配文档名称而复制另一份 actor、turn loop 或 Port。

### 2.4 `nomifun-coding-runtime`

只拥有 Coding profile：

- Coding instructions；
- Workspace/AGENTS.md 发现；
- model-facing Coding Tool schema；
- model tool → canonical capability/action 映射；
- Process/Patch/VCS workflow；
- diff/review/test workflow；
- Coding feature inventory。

它是一个具体的 Coding Engine 实现层，不是 Plugin。未来其他 Engine 可以复用
同一个 Port 合同，但不需要把旧 Engine 改造成 Coding Engine。

当前 `nomifun-coding-engine` 同时承载前述通用最小 loop 和 Coding profile seam。
当 Process/Patch/Context 等切片增长到独立 owner 边界时，再在 `CAR-04`～`CAR-06`
按依赖方向拆出模块；拆分必须保持行为测试和 public Port 不变。

### 2.5 现有 NomiFun owner

| 事实/能力 | Owner |
|---|---|
| AgentPreset/Revision/Snapshot/Binding/Session | `nomifun-agent-platform`、`nomifun-agent-session` |
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

`nomifun-coding-engine` 已在自己的 `CodingModelPort` 和 `CodingToolInvoker` Port 上
传递 `CancellationToken`，并在 turn loop 内返回明确 `Cancelled` 终态。但是当前
平台合同仍有两个待远程接线的缺口：

- `ChatBrokerPort::open_chat_stream` 没有 cancellation 参数；当前 adapter 只能停止
  转发和消费，不能保证底层 Provider 请求立即终止；
- 现有 Capability handler 调用合同没有 cancellation 参数；Process/SSH/MCP 等 owner
  需要在 `CAR-03`/`CAR-04` 明确取消传播和清理报告。

在这些合同补齐前，不能把“停止消费”宣称为端到端取消完成；当前 Kernel adapter
的取消是 invocation future 的 fail-fast，owner handler 的真实停止仍由中央
Capability cancellation 合同保证。

## 4. 依赖图约束

允许：

```text
Agent Platform → Agent Runtime → narrow Port
Agent Runtime → Coding Runtime
Agent Runtime → ChatModelBroker contract
Agent Runtime → Kernel invocation contract
Coding Runtime → File/Process/VCS contracts
```

禁止：

```text
Agent Runtime → root SQLite
Agent Runtime → Codex API/Auth/ModelProvider
Agent Runtime → app-server protocol
Agent Runtime → concrete Plugin/Browser/Computer instance
Agent Runtime → old ConversationService / NomiAgentManager
```

## 5. 一个 Engine Binding/Runtime Session 的生命周期

```text
用户选择 Engine Family/Channel 或 exact Build
  → Resolver 解析 exact Engine Build
  → 将 Engine Build 写入新 Session Binding
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
