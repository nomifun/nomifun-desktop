# CAR 模型与 Provider 适配方案

## 1. 目标

本文件只解决“内化后的 Agent Runtime 如何调用模型”。它不重新设计 NomiFun
模型平台，也不把 Codex 的模型/认证层搬进来。

唯一生产路径：

```text
AgentSession exact EngineBinding
  → selected Coding Engine Build
  → RuntimeSessionActor
  → AgentModelPort
  → nomifun-chat-model-broker::ChatBrokerPort
  → NomiFun Provider Adapter
```

模型 route 和执行 Engine 是两个正交选择：

- `EngineBinding` 决定哪个进程内执行引擎实现当前 Session；
- `ChatRouteIdentity` 决定该 Session 当前 Snapshot 允许的模型 route；
- Engine Build 失败不能触发 Engine fallback；
- Broker 只可在既有 route policy 和 pre-semantic-output 边界内重试/failover；
- 无论模型 route 如何重试，同一个 Session 的 exact Engine Build 不变。

## 2. 复用现有 NomiFun 合同

优先复用现有类型和语义：

| Runtime 需要的事实 | NomiFun 类型/能力 |
|---|---|
| 精确模型路由 | `ChatRouteIdentity`、`ResolvedChatRoute` |
| 请求输入 | `ChatModelRequest`、`ChatModelInput` |
| 工具定义 | `ChatToolDefinition`、`ChatToolChoice` |
| Tool Call | `ChatToolCall`、`ChatContentPart::ToolCall` |
| Tool Result | `ChatContentPart::ToolResult` |
| Reasoning | `ChatContentPart::Reasoning`、`ChatModelEvent::ReasoningDelta` |
| 流式事件 | `ChatModelEvent`、`BrokerEventEnvelope` |
| 因果校验 | `ChatCausality`、`ChatCausalityGate` |
| Credential | `CredentialLease`、`ProviderCredentialStore` |
| 重试/故障 | `ChatRetryDirective`、`ChatModelError` |
| 协议适配 | `ChatProtocolAdapter` |

不得复制一套与 `ChatModelInput`、`ChatModelEvent` 平行的长期 Provider DTO。
如果 Runtime 需要额外信息，应使用进程内 wrapper 或独立的非序列化控制参数。

建议 Port：

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

`CancellationToken` 不进入 JSON、SessionEvent、Prompt 或模型请求 metadata。

## 3. Codex 概念到 NomiFun 概念

| Codex 概念 | NomiFun 适配 |
|---|---|
| `ModelClient` / `ResponseStream` | `AgentModelPort` / Broker stream |
| Codex ModelProvider | `ChatRouteResolver` |
| Codex AuthManager / CodexAuth | `ProviderCredentialStore` + opaque `CredentialLease` |
| Codex Responses input | `ChatMessage` / `ChatContentPart` |
| Codex Responses delta | `ChatModelEvent` |
| Codex provider retry | Broker 的 `ChatRetryDirective` |
| Codex response/thread ID | Runtime transient `ProviderResponseId` / `ProviderRoundId` |
| Codex compact endpoint | NomiFun typed `AgentCompaction` task |

Runtime 不知道当前请求使用 Anthropic、OpenAI Chat、OpenAI Responses、Gemini、
Bedrock 还是 Vertex。新增 Provider 只修改 Broker adapter 和 route resolver。

## 4. Coding 请求合同

### 4.1 普通 Agent Turn

```text
AgentSession facts
  + exact Snapshot/Binding
  + reconstructed history
  + current user input
  + selected Tool definitions
  + selected reasoning policy
  → ChatModelRequest
  → Broker stream
```

请求必须携带并校验：

- `agent_session_id`；
- `turn_operation_id`；
- `causation_event_id`；
- `resolved_snapshot_ref`；
- exact `EngineBinding` 中的 `agent_session_id`、Build digest 与 Snapshot 必须匹配；
- exact `ChatRouteIdentity`；
- 当前工具 schema；
- 当前 Snapshot 允许的 tool call history。

当前隔离实现已在 `CodingTurnRequest` admission 校验 Session 与 Snapshot 是否匹配
固定 `EngineBinding`；远程接入时还必须由 AgentSession application service 校验
`runtime_binding_id` 和 SessionEvent 中的 runtime-bound 事实。

### 4.2 Compaction Turn

Compaction 使用 NomiFun 自己的 task identity：

```text
model_task = agent_compaction
tools = []
response_format = text
input = bounded context selected by Runtime
```

它可以复用主 route，也可以使用用户或平台显式配置的 compaction route；每次调用
开始前解析并冻结。禁止调用 Codex 私有 `/responses/compact`、`/compact` 或
Codex 专属 backend。

## 5. Feature gate

### 5.1 Coding P0

Coding route 至少需要：

- text input；
- text output；
- Tool Calls；
- bounded streaming；
- cancellation；
- 可表达 `tool result` 的输入语义。

缺少任一项时，Compiler 或 Runtime admission 返回 typed unavailable，不静默
降级为普通 Chat，也不自动换到 Snapshot 外的模型。

### 5.2 可选能力

| 能力 | 默认策略 |
|---|---|
| Reasoning | 如果 Revision 选择且 route 支持则启用；缺失时 fail closed |
| Reasoning signature | 只有 provider 和 Snapshot 都支持时保留 |
| Native Responses Items | 可选，不能成为通用 Runtime 必需项 |
| Provider Round State | 仅在 continuation 需要且 route 支持时启用 |
| Prompt Cache | 由 Broker 处理，不改变语义 |
| Image input | 由 Snapshot 明确选择，不属于 Coding 基线 |
| Audio / Realtime | 不进入 CAR Coding Runtime |

六种现有 Chat protocol 仍由 Broker 统一维护，但不是每个 route 都必须具备所有
高级特性。route feature filtering 必须发生在 Broker/Compiler 的确定性边界。

## 6. 重试、Failover 与语义边界

职责严格分开：

```text
Broker:
  credential lease
  provider adapter
  route attempt
  pre-semantic-output retry/failover
  usage/error normalization

Runtime:
  model step continuation
  Tool Call continuation
  tool result injection
  compaction trigger
  turn cancellation
```

以下情况不得由 Broker 或 Runtime 自动切换另一个 route：

- 已产生 semantic output；
- 已提交 Tool Call；
- 已开始 managed effect；
- 已开始 external uncertain effect；
- 当前 route 已被 Session 因果链记录。

Provider stream 在首个 semantic event 前失败时，可按 Broker policy retry/failover；
其余情况写入单一 terminal failure，由 owning domain 决定是否 reconcile。

## 7. 取消与 Backpressure

- Runtime 创建 per-turn cancellation token；
- Broker adapter 必须能够停止底层 stream；
- Runtime 停止消费后，不能无限缓存 provider event；
- stream channel 有固定上限，满载时按取消/失败语义收敛；
- cancellation 不得被转换成可重试的 `ProviderUnavailable`；
- cancellation 后不得隐式发起下一次 route attempt。

当前 `ChatBrokerPort::open_chat_stream` 尚未接收 cancellation token。隔离 adapter
可以立即停止向 Coding Engine 转发和消费，但不能证明 Provider HTTP 请求已立即
终止。端到端取消传播是 `CAR-02` 的远程中央合同修改，不得在交接报告中标成已完成。

## 8. 测试闭环

至少覆盖：

1. exact route/causality 校验；
2. CredentialLease target mismatch；
3. text + Tool Call + Tool Result 多轮 continuation；
4. reasoning delta 和 signature；
5. provider stream 在 semantic output 前失败时的有限 failover；
6. semantic output 后不 failover；
7. cancellation 传播到 adapter；
8. bounded stream/backpressure；
9. compaction 使用 NomiFun task，不调用 Codex endpoint；
10. Provider-specific metadata 不泄漏 secret/header/raw URL。
