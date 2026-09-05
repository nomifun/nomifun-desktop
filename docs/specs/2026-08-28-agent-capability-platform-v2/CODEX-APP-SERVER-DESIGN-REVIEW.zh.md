# Codex app-server 方案设计复盘

> 审阅日期：2026-09-03
>
> 审阅分支：`rf/agent-capability-platform-v2`
>
> 审阅基线：当前工作树 `850cccc8a1d3`。工作树在审阅时存在此前未提交的其他改动；本文只描述实际源代码形态，不把当前工作树当作发布候选。
>
> 审阅限制：本轮只读仓库代码和已有审计记录；没有构建或运行 Codex，没有运行 live provider，也没有读取、使用或写入任何 API key。

## 2026-09-03 用户确认的当前执行修订

本节是对下方初始审阅快照的当前执行覆盖。初始审阅中的 Sidecar 动机、协议缺口、
兼容性分析和前置条件继续保留，供未来重新接入 Codex app-server 时复用；但当前阶段
不再把 Codex app-server 当作产品执行内核或发布前提。

### 当前产品路径

```text
Web / Desktop / nomicore
  -> NomiCoreApplication
  -> AppServices / ConversationService / AgentRuntimeRegistry
  -> nomifun-ai-agent / nomifun-conversation / nomi-agent
  -> Nomi engine
```

- 本阶段唯一产品执行内核是 NomiFun 原有 Nomi engine。
- Web、Desktop `start_with_outcome` 和 `nomicore` 默认服务入口均选择
  `NomiCoreApplication`。
- 当前 Chat、Coding、Provider/Model、工具、历史、stream、cancel、artifact 和
  session persistence 的验收都必须经过该 Nomi-core 产品路径。

### 保留的低成本多-runtime host boundary

```text
current default product host
  -> NomiCoreApplication

future explicit runtime host
  -> FreshV4Application
  -> Codex app-server or another Runtime integration
```

该边界只保留两个独立 composition root，不建设动态 Engine catalog、运行中切换、
per-turn 切换或自动 fallback。一个 `AgentSession` 绑定一个明确的 host；host 不可用时
显式失败或保持可读，不迁移到另一条 Runtime 主链。

保留这个边界的成本较低，因为 AgentSession、Snapshot、Role/Provider、Capability、
Remote 和 Model Broker 等 Host-owned 合同本来就不应属于某个具体 Runtime。未来
Codex app-server 可以在 `FreshV4Application` 后重新接入，但必须满足同一 canonical
Session/Provider/Capability 边界，不能直接读取 `AppServices`、Provider 数据库或业务
service bag。

### 当前 Codex 结论不变

- 当前 `nomifun-codex-runtime` 是 NomiFun 侧的协议/client/supervisor/adapter 和
  外部进程管理代码，不是移植进 NomiFun 的 Codex engine。
- fixture、synthetic contract、adapter mapping、Host Broker smoke 和 opening/process
  cleanup 只能证明各自局部行为，不能证明 Codex-native 已移植、完整 Coding 已完成或
  Remote/AgentExecution/IDMM 已由 Codex 承载。
- 未来若继续使用外部 app-server，应明确命名为 external Codex integration；若要宣称
  Codex-native，则 Codex source 或受控 workspace dependency 必须真实进入构建图。

## 结论摘要

### 结论一：原方案的设计动机是合理的，但它解决的是“外部 Runtime 接入”问题

此前的 `Codex app-server` 方案采用：

```text
NomiFun Host
  -> AgentSession / Snapshot / Capability authority
  -> Host Broker
  -> external Codex app-server Sidecar
       stdin/stdout newline-delimited JSON
```

这个方案主要试图解决以下问题：

1. 把 Codex 进程故障、子进程和平台差异隔离在 Host 之外；
2. 让 Host 继续掌握 AgentSession、权限、资源绑定、Provider 凭据和持久化事件；
3. 通过 stdio 建立本机边界，避免额外监听端口和网络服务暴露面；
4. 对 Sidecar 可执行文件做目标平台、来源、digest 和进程树生命周期约束；
5. 在 Host 侧复用现有 Provider/Model Broker，而不是让外部进程直接接触 NomiFun 的数据库和密钥。

这些目标可以解释为什么当时选择 Sidecar 和 bridge。

### 结论二：它不满足“把 Codex 代码移植进 NomiFun 内核作为核心 Coding runtime”的预期

明确结论是：**不满足。**

当前实现不是 Codex 源码移植进 NomiFun 内核，也不是把 Codex engine 编译进 `nomifun-ai-agent` 或 `nomifun-conversation`。当前实际形态是：

```text
NomiFun 自己实现的 protocol client / supervisor
  + 外部可执行 Runtime artifact
  + Host 侧 ChatModelBroker bridge
```

其中：

- `nomifun-codex-runtime` 是本地的协议、进程和生命周期管理 crate；
- `vendor/codex-runtime` 是来源、协议和合同元数据目录，明确声明不复制 upstream source；
- 真正被启动的是由路径解析得到的外部可执行文件；
- 当前 `RuntimeStartTurnBrokerBridge` 在 Host 进程内调用 `ChatModelBroker`，并不是把完整 Codex 模型循环放在 NomiFun 内核中；
- 现有生产 Nomi 执行路径仍由 `nomifun-ai-agent`、`nomifun-conversation`、`nomi-agent` 和 `AgentRuntimeRegistry` 负责。

因此，不能把这套实现命名为“Codex-native 已完成”，也不能把它当作 Nomi 内核已经被 Codex 替换。

### 结论三：它可以与重构接口形成部分连接，但不是完整兼容

当前 Host 已经建立了若干可复用的连接点：

- `AgentPlatform` 可以从保存的 `AgentBinding` 编译 Snapshot；
- Runtime command 携带 `AgentSessionId`、Snapshot ref、Runtime profile digest 和 active-set generation；
- Host 可以把 Runtime 生命周期写入 `SessionEvent`；
- Provider/Model route 可以经由 Host Broker 解析；
- Remote 可以复用 canonical AgentSession command/query ports。

但是这些连接点多数是 Host 侧的合同和适配，不代表 Sidecar 已经理解或执行 NomiFun 的 Capability、Role、Provider、Snapshot、AgentExecution 或 IDMM 语义。完整 Coding 主链仍然缺少工具回调、历史恢复、事件语义、取消终态和真实协议接入。

### 结论四：本期应回到 Nomi 内核完成原重构计划

本期的可交付方向应是：

```text
AgentSession
  -> canonical Snapshot / Role / Capability authority
  -> Nomi-backed runtime adapter
  -> nomifun-ai-agent / nomifun-conversation 原有 Nomi engine
  -> Provider/Model 管理与真实工具链
```

Codex-native 应延后到二期或三期。重新启动前必须先证明“源码确实进入构建图、模型与工具只有一个执行所有者、事件和资源生命周期能完全映射到 canonical AgentSession”，而不是继续增加 fixture、adapter 或外部包装层。

## 现状事实

### 1. Codex 源码没有被当前 Runtime crate 编入

当前 workspace 的 Cargo 配置把 `nomifun-codex-runtime` 作为本地 crate 纳入，但没有把外部 `../codex` checkout 作为 workspace member，也没有在该 crate 的 `Cargo.toml` 中声明 Codex engine 或 app-server source dependency。`Cargo.toml:1-3, 48-62` 和 `crates/backend/nomifun-codex-runtime/Cargo.toml` 显示，该 crate 的依赖主要是：

- `nomifun-agent-contracts`；
- `nomi-process-runtime`；
- `tokio`、serde、digest 和错误处理库。

`vendor/codex-runtime/README.md` 明确说明该目录“intentionally does not copy the upstream source tree”。`vendor/codex-runtime/source-lock.json` 的事实字段为：

```json
{
  "repository_alias": "../codex",
  "vendored_upstream_source": false,
  "reviewed_paths": "...",
  "upstream_transport": "stdio"
}
```

并且 `upstream_source_files_in_vendor` 为空。仓库当前保存的是外部 checkout 的来源记录，不是 upstream 源码副本。

`vendor/codex-runtime/patches/series.json` 列出了 `runtime/hello`、stable RPC、FullAuto、native action ACK、credential handle 和 dispose 等意向 patch，但该 JSON 是 patch 目录的描述，不是 Codex 源文件已经应用并进入本仓库构建的证据。不能用 patch 名称代替实际 source、build 和 artifact provenance。

### 2. 当前 Sidecar 是外部进程

`crates/backend/nomifun-app/src/bootstrap/runtime_artifact.rs:26-52` 解析目标平台的 Runtime artifact，读取外部 executable 及其旁边的 hello metadata，然后交给 Host。

`crates/backend/nomifun-codex-runtime/src/process.rs:19-53, 282-354` 规定并启动：

```text
app-server --listen stdio://
```

该过程具有以下特征：

- executable 必须是绝对路径；
- 生产路径要求 regular、non-symlink、non-reparse 文件；
- 生产路径检查 executable SHA-256；
- 环境变量会被清理，只恢复有限的非密钥环境；
- 凭据通过一次性的继承句柄/匿名通道传输，而不是通过 argv、环境变量或普通 JSON；
- Host 保存并在 dispose 时关闭整个 descendant process tree。

这些是一个外置进程管理器的特征，而不是一个嵌入式 engine。

### 3. 当前 production composition 同时存在两条不同 Runtime 组合线

> 初始审阅快照说明：下文记录的是 `FreshV4Application` 与原 Nomi 组合曾同时存在于
> 仓库中的结构事实。2026-09-03 当前执行修订后，Web、Desktop 和 `nomicore` 默认入口
> 已统一选择 `NomiCoreApplication`；Fresh-v4/Codex 线只保留为显式未来 host，不是
> 当前默认 production path。

Fresh-v4 canonical Host 在 `agent_platform_host.rs:1593-1640` 组合：

```text
ChatBrokerHostComposition
  -> ChatModelBroker
CodexRuntimeSupervisor
  -> SupervisedCodexRuntimePort
RuntimeStartTurnBrokerBridge
  -> BrokerBackedRuntimePort
AgentPlatform
```

这条线用于新 AgentSession/Remote 组合，但 `CodexRuntimeSupervisor` 的构造本身是惰性的，只有实际 Runtime admission 才会启动子进程。

与此同时，`AppServices` 在 `services.rs:3017-3095` 仍然创建：

```text
AgentFactoryDeps
  -> build_agent_factory
  -> InMemoryAgentRuntimeRegistry
  -> NomiAgentManager
```

`nomifun-ai-agent/src/factory/mod.rs:188-254` 的 factory 当前把生产 Agent 构造分派到 `nomi::build`。`AgentRuntimeHandle` 的生产变体仍是 `Nomi(Arc<NomiAgentManager>)`；另一个 `Mock` 变体只在测试支持配置下存在，见 `nomifun-ai-agent/src/runtime_handle.rs:150-181`。

因此，当前不能说 Codex Runtime 已经替换 Nomi。更准确的描述是：仓库保留一条
尚未完成的 Codex-derived Runtime 研究线作为未来显式 host；当前三个产品入口已经
统一选择 NomiCoreApplication，Nomi runtime 是本阶段唯一产品执行内核。

### 4. Host Broker bridge 的实际行为比“完整 Coding Runtime”窄

`crates/backend/nomifun-agent-platform/src/runtime_chat_bridge.rs:1-8` 直接说明：Sidecar 先接受 `StartTurn`，然后 Host 构造一次无状态的 Responses 请求，调用注入的 `ChatModelBroker`，再把结果写回 Session。

`responses_request` 位于 `runtime_chat_bridge.rs:692-725`，当前实际请求特征是：

- route identity 来自已提交的 turn fact；
- instructions 来自有限的 context contributions；
- input 只有一条用户文本消息；
- `tools` 为空；
- `tool_choice` 为 `None`；
- `previous_response_id` 为 `None`；
- `preserve_native_responses_items` 为 `false`；
- 输出被限制为文本和固定上限；
- 结果由 Host 追加 `message/content-part`、`message/completed` 和 `turn/completed`，或 `turn/failed`。

`BrokerBackedRuntimePort` 在 `runtime_chat_bridge.rs:867-927` 中先调用底层 Runtime command，收到 `StartTurn` 成功结果后再异步启动 Host bridge。也就是说，当前 Sidecar 的 `StartTurn` acknowledgement 和实际模型调用是两个执行边界。

这可以作为最小 Chat 骨架或受限验证桥，但不能证明 Codex app-server 正在执行完整 Coding loop。

### 5. 当前 Nomi 内核已经拥有更完整的运行时职责

`nomifun-ai-agent/src/manager/nomi/agent.rs:203-300, 910-1320` 的 `NomiAgentManager` 持有并协调：

- `nomi_agent::engine::AgentEngine`；
- Nomi `Session` 和 `SessionManager`；
- Provider 配置与模型；
- MCP manager；
- 文件、终端、知识、伙伴、Cron 等工具；
- Browser lane、SSH lease 和 loopback capability lease；
- 输出、artifact、stream 和 turn cancellation；
- Nomi session persistence。

`nomifun-ai-agent/src/runtime_registry.rs:58-75, 467-545` 还负责每个 Conversation 的 single-flight runtime slot、模型配置绑定、workspace binding、崩溃重启 governor、teardown quarantine 和 Nomi session persistence。

这套内核路径与 `nomifun-codex-runtime` 的职责不同。前者是实际 Agent engine 和工具生命周期，后者是外部进程的协议和进程监督。

### 6. 现有验证证据的边界

截至 2026-09-03，已有台账记录的验证可以证明若干局部事实，但不能扩大解释为 Codex-native 完成：

| 证据 | 能证明什么 | 不能证明什么 |
| --- | --- | --- |
| `nomifun-codex-runtime` 单元测试 | JSONL client、句柄通道、超时、进程树清理和合同校验的 fixture 行为 | Codex 源码已经被移植、真实 app-server 协议可用、完整 Coding 可用 |
| `chat_minimal` | Node runtime fixture 加 recorded provider transport 可以走一条最小 Session/turn/receipt 路径 | 真实 Codex binary、真实模型、工具循环、历史恢复和产品 Chat |
| `coding_codex` | 合同校验器可以检查 synthetic Snapshot、Profile、事件和生命周期形状 | 真实 Coding runtime 已执行文件、终端、VCS、MCP、Review 和 Test |
| `PinnedAppServerAdapter` 测试 | stable method 到 upstream method 的纯映射和策略覆盖逻辑 | 该 adapter 已接入 production Client/Supervisor，或真实 binary 已按 upstream 协议工作 |
| StepFun live broker smoke | Host `ChatModelBroker` 能否直接调用某个 Provider route | Sidecar 是否执行模型调用、工具回调、取消或产品级 Chat/Coding |
| Remote REST E2E | 无 Sidecar 时 Remote open 能够持久化并 fail-closed | ready 状态下真实 `open -> turn -> observe -> cancel` 已通过 |
| `bun run dev` | Desktop Host 和 UI 可以启动到可观察页面 | Coding runtime、模型调用、工具权限和跨平台发布已经完成 |

已有 live broker smoke 曾记录 Provider `503`/`ProviderUnavailable`。这只能作为当次 Provider 或网络结果记录，不能被改写为 Codex runtime 通过或失败；本复盘没有重新执行该测试。

## 原方案为何采用 Sidecar、stdio 和 Host Broker bridge

### 1. 外部 Sidecar 的目标

当时的设计假设是：Codex app-server 是一个已有自己的 session、thread、turn、工具和模型循环的独立 Runtime。把它放在进程外可以获得：

1. **故障隔离**：Codex 或其子进程崩溃时，尽量不直接破坏 NomiFun Host 的数据库和 UI；
2. **平台隔离**：Windows、macOS 和 Linux 可以分别交付目标 Runtime artifact；
3. **来源约束**：通过 pinned source、fork commit、protocol digest 和 executable digest 约束可执行文件；
4. **生命周期边界**：由 Host 统一关闭 stdin、等待 bounded deadline，并清理整个进程树；
5. **权限收窄**：禁止 Sidecar 自己接收交互审批请求，固定 FullAuto 策略，避免未授权的审批旁路；
6. **依赖解耦**：Host 不需要把 Codex 的内部类型直接扩散到整个 NomiFun 业务图。

这些目标对接入一个成熟的外部 Runtime 是合理的工程考虑。

### 2. stdio 的目标

已有 upstream spike 记录表明，指定 pinned upstream 的 app-server 使用 newline-delimited JSONL，连接先进行：

```text
initialize
  -> initialized notification
  -> thread/start | thread/resume | thread/fork
  -> turn/start | turn/steer | turn/interrupt
```

stdio 因此有三个直接收益：

- 本机通信不需要开放 HTTP 或 websocket 端口；
- Host 可以把输入、输出、EOF 和进程生命周期绑定在同一个 child process；
- 协议流可被限制为单一 JSONL reader/writer，并设置 frame size、in-flight request 和 timeout。

不过，stdio 只说明传输方式，不说明 Runtime 是否已经内嵌，也不自动解决协议、权限、工具和历史语义的兼容。

### 3. Host Broker bridge 的目标

Host Broker bridge 的原始意图是把两个职责分开：

```text
Codex Runtime
  负责 Agent turn 的外部 Runtime 生命周期

NomiFun Host
  负责 Provider route、credential、Session facts、Capability authority 和持久化
```

理论上，这样可以让同一个 NomiFun Provider/Model 管理系统服务多个 Runtime，而不让每个 Runtime 直接读取 Provider 数据库或持有长期密钥。

当前实现确实体现了这一意图：

- `AgentPlatform` 组合 `ChatBrokerPort`；
- `runtime_chat_bridge` 从 Session fact 中读取 exact route identity；
- Provider credential 仍由 Host-owned Broker lease 处理；
- Sidecar 只接收一次性 Runtime credential handle，不接收 Provider API key；
- Host 将结果写入 canonical Session event。

但这一方案的代价是模型执行所有者被拆成两层。只要 Sidecar 不是完整的 Codex engine，bridge 就会退化为“Sidecar 接受命令，Host 直接调用模型”。这正是当前实现与用户预期不一致的地方。

## 接口连接方式

### 1. AgentSession 连接

当前 canonical 流程大致如下：

```text
OpenAgentSessionRequest
  -> AgentPlatform::open_session
  -> compile_saved_binding
  -> AgentSessionStore::create_session
       session/opening
       capability/active-set-committed

AgentPlatform::launch_session_runtime
  -> derive RuntimeCommandContext
  -> CodexRuntimePort::launch
  -> external Runtime handshake + create
  -> runtime/bound
  -> session/ready

AgentPlatform::start_turn
  -> message/user-accepted
  -> turn/started
  -> RuntimeCommand::StartTurn
  -> Host Broker bridge
  -> message/content-part
  -> message/completed
  -> turn/completed / turn/failed
  -> read_turn_receipt
```

对应代码事实：

- `AgentSessionLiveRecord` 保存 Session owner、`AgentBindingValue` 和 Remote provenance，见 `nomifun-agent-contracts/src/session.rs:21-35`；
- `RuntimeCommandContext` 保存 `agent_session_id`、`runtime_binding_id`、`operation_id`、Snapshot ref、profile digest 和 active-set generation，见 `nomifun-agent-contracts/src/runtime.rs:288-297`；
- `AgentPlatform::start_turn` 在 `platform.rs:3267-3515` 先写入 Session facts，再发送 Runtime command；
- Runtime 成功后，`commit_runtime_binding` 会把 `runtime/bound` 和 `session/ready` 写回 Session，见 `platform.rs:2982-3067`；
- `AgentSessionQueryPort::read_turn_receipt` 只读 durable turn terminal，见 `platform.rs:423-459` 和 `platform.rs:3918-3929`；
- cancel 会先进行 Session 侧的 durable admission，再发送 Runtime cancel，见 `platform.rs:2381-2442`。

因此，AgentSession 是 Host 的权威身份和持久化边界；Sidecar 不是 Session store，也没有权力自行改变 Session owner、Snapshot 或 active-set。

### 2. Capability 和 Role 连接

`ResolvedSnapshotContent` 当前冻结：

- required Runtime protocol/profile；
- Runtime feature inventory；
- model route；
- initial/on-demand capabilities；
- activation plans；
- capability allowlist；
- skill locks；
- MCP tool locks；
- resolved Role Provider locks；
- typed resource bindings；
- schema 和 target contribution digest。

这些字段位于 `nomifun-agent-contracts/src/preset.rs:392-418`。

`AgentPlatform::pinned_runtime_profile` 和 `runtime_create_command` 把其中一部分投影为 `PinnedRuntimeProfile` 与 `RuntimeCreateParams`，见 `platform.rs:2221-2287`。Sidecar 收到的是 capability ID、Runtime feature、profile digest 和 typed resource binding 描述，不是 Kernel 中的具体 handler、repository 或资源对象。

真正的 Host capability 调用仍由：

```text
AgentPlatform::invoke_capability
  -> ThinAuthority
  -> KernelRegistry
  -> Role Provider lock
  -> exact capability/action handler
```

完成，见 `platform.rs:3607-3833`。这条路径与 Sidecar 的自定义 `native_action/start` 并不是同一个完整调用循环。

### 3. Provider 和多模型连接

当前 Provider 管理存在两套可观察的 Host 侧能力：

1. canonical v4 `ChatBrokerHostComposition` 从 provider、provider_models、provider_model_capabilities、provider_connections 和 route record 解析 exact `ResolvedChatRoute`；
2. Nomi factory 经 `ModelInvokeService` 的 task/connection resolver 得到 `NomiResolvedConfig`，再由 `nomi_providers::create_provider` 和 `AgentEngine` 执行。

Nomi 路径的解析逻辑见 `nomifun-ai-agent/src/factory/provider_config.rs:118-302`，其中 protocol 选择 serializer 和 endpoint，credential 由 Host 解密并装入 Nomi config。

Codex bridge 理论上可以使用第一套 Broker：

```text
AgentSession Snapshot
  -> ChatRouteIdentity
  -> ChatBrokerHostComposition
  -> Provider connection / credential lease
  -> model stream
```

当前实际也部分如此。但 `RuntimeCommandContext` 本身没有完整 Provider connection、credential lease 或模型 serializer；它只引用 Snapshot 和 operation identity。真正的模型请求由 Host bridge 重新组装，而且目前只覆盖 text Responses 子集。因此它不能替代 Nomi factory 的完整 Provider/Model/Tool 语义。

### 4. Snapshot 连接

Snapshot 的作用是冻结一次 Session 的执行闭包，而不是存放当前 latest 配置。Host 在打开 Runtime 时会校验：

- 传入 Snapshot ref 是否等于已编译 Snapshot；
- Runtime profile digest 是否一致；
- active-set generation 是否一致；
- Runtime binding 的 Session identity 是否一致；
- Role/Capability/resource 绑定是否属于同一 owner。

`AgentPlatform::compile_saved_binding` 在 `platform.rs:2514-2608` 从持久化 revision 和 Snapshot 重新编译并检查 convergence；`validate_compiler_convergence` 在 `platform.rs:4068-4098` 比较 persisted Snapshot 与 Kernel compiler 结果。

这是一个适合任何 Runtime executor 的接口。但当前 Codex Sidecar 只消费其中有限的 metadata，尚未真正执行 Snapshot 中所有 capability、tool、skill、MCP、Role Provider 和 resource contract。

### 5. Runtime 生命周期连接

`CodexRuntimeSupervisor` 负责：

- per-binding admission；
- opening registry；
- Sidecar process spawn；
- hello expectation；
- Runtime open timeout；
- inbound event task；
- dispose RPC；
- process-tree cleanup。

`RuntimeIngressPort` 允许 Sidecar 向 Host 发两类当前自定义请求：

- `runtime/event`；
- `native_action/start`。

Host 在 `AgentPlatform` 中把 Runtime event 追加到 Session，并对 `native_action/start` 做 identity、Snapshot、generation 和 effect-start 校验，见 `platform.rs:4232-4388`。

这使 Runtime lifecycle 能够挂到 Session event log，但它仍是当前自定义合同，不等于 upstream app-server 的原生事件和工具协议已经被接通。

### 6. Remote 连接

Remote REST 在 `remote_rest.rs:73-79` 持有：

- canonical Session command port；
- canonical Session query port；
- `RemoteRuntimeCoordinator`。

Remote open 先提交带 `RemoteBindingProvenance` 的 AgentSession，再由 coordinator 异步进行 Runtime admission。`remote_runtime.rs:355-418` 解析外部 Runtime artifact、生成一次性 Runtime credential、建立 working directory，并调用 `AgentPlatform::launch_session_runtime`。

Remote turn/observe/cancel 通过 Session command/query ports 和 operation-specific deadline 执行，见 `remote_rest.rs:463-720`。这说明 Remote 可以复用 Host 的 canonical Session API。

在初始 Fresh-v4/Codex 路径中，Remote ready-chain 依赖 exact Sidecar、真实模型和安装
token；无 packaged Sidecar 时得到 `open_failed` 只能证明 fail-closed。2026-09-03
当前执行修订后，产品 Remote 应改走 NomiCore/Nomi engine，不再等待 Codex Sidecar；
仍需真实 Provider、installation token 和 `open -> ready -> turn -> observe -> cancel`
产品验收。

## 多模型、AgentExecution、IDMM 和生命周期的理论桥接

### 1. 多模型和 Provider 管理

#### 理论上如何桥接

若 Codex engine 真正作为一个 Runtime executor 接入，理想链路应当是：

```text
AgentBinding
  -> ResolvedSnapshot
  -> exact ChatRouteIdentity / ModelRouteId
  -> Host Provider resolver
  -> credential lease
  -> Runtime model port
```

这里有两个可接受的模型所有权选择：

1. Codex engine 自己执行模型请求，但只能通过 NomiFun 提供的 typed model port，不能自行读取 Provider DB 或重新解析 latest；
2. Host Broker 执行模型请求，Codex engine 只负责纯编排，但这时必须承认它不是完整 Codex model runtime，并明确其工具、历史和事件能力边界。

两种选择不能在同一个 turn 中同时成为隐含的模型所有者。

#### 当前实际缺口

- 当前 Sidecar 的 Runtime command 没有完整 Provider/connection/model invocation contract；
- Host bridge 直接构造受限 text request，绕过了完整 Coding tool/history 输入；
- Provider route identity 虽然被写入 Session fact，但不是 Sidecar 内部可执行的 model binding；
- live broker smoke 没有经过 AgentSession、Runtime Supervisor 和工具回调；
- Provider route 改变时，Nomi registry 已有 model-config binding 和 runtime recycle 机制，而 Codex Runtime 没有等价的完整 provider binding 生命周期。

### 2. AgentExecution 和 Agent 集群

#### 理论上如何桥接

现有 AgentExecution 的合理 canonical 映射应为：

```text
AgentExecution
  -> participant / step / attempt
  -> one AgentSession per attempt
  -> attempt-specific AgentBinding + frozen Snapshot
  -> canonical start_turn
  -> operation-scoped TurnReceipt
  -> artifact / token / error projection
  -> scheduler settles the attempt
```

Scheduler 仍然可以拥有 DAG、lease、retry policy 和 participant orchestration；它不应直接拥有 Runtime process 或自行构造 Conversation。每个 attempt 的 Runtime、工具和 Provider 由 AgentSession/Kernel 侧统一授权。

#### 当前实际缺口

`nomifun-agent-execution/src/attempt_runner.rs:141-380` 虽然已经定义了 `AgentExecutionSessionPort`，但生产实现 `ConversationExecutionSessionPort` 仍包着：

- `ConversationService`；
- `AgentExecutionConversationPort`；
- `AgentRuntimeRegistry`；
- `ConversationResponse` 和旧 `SendMessageRequest`；
- `AgentExecutionTurnAuthority`。

`state.rs:1400-1448` 也明确把它标记为 transitional adapter。当前还缺：

- AgentExecution 到 canonical AgentSession 的真实 create/attempt adapter；
- Fresh-v4 中与 execution/attempt 绑定的完整持久化组合；
- canonical artifact、token、error 和 attempt receipt projection；
- canonical steer/continue/failover 映射；
- scheduler 与 Runtime binding 的单一 owner；
- 一个生产可用的 Codex 或 Nomi 通用 AgentSession executor 集群接口。

所以，现有 AgentExecution 的 typed facade 只能说明边界正在收缩，不能说明 Codex 已经能够承载 Agent 集群。

### 3. IDMM

#### 理论上如何桥接

IDMM 若迁移到 canonical Session，应当使用：

```text
AgentSessionQueryPort
  -> Session head / event cursor / durable receipt
AgentSessionCommandPort
  -> exact turn continuation / cancel / failover command
Kernel authority
  -> effect reservation / resource binding
Provider route
  -> exact backup model selection
```

IDMM 的 observer 可以读 Session event，而不是猜测文本或 idle；介入动作必须绑定一个 exact active turn operation，并由 owning Session command port 处理。

#### 当前实际缺口

`nomifun-idmm/src/probe.rs:207-328` 当前的 `ConversationSessionPort` 仍依赖：

- `ConversationService`；
- `AgentRuntimeRegistry`；
- `broadcast::Receiver<AgentStreamEvent>`；
- `ConversationRuntimeSummary`；
- 旧 `IdmmTurnScope`；
- `idmm_continue_active_turn` 和 `idmm_failover_conversation`。

`state.rs:1451-1490` 还单独构造 `nomifun_idmm::SidecarClient` 和 `LiveCompleter`。这个 IDMM sidecar 是一个 Host 侧一次性备用模型 completion helper，不是 Codex app-server Sidecar；两者不能混称。

当前缺少 canonical AgentSession 的 live subscription primitive、active-turn scope、same-turn failover 和 backup route contract。因此 IDMM 尚未能证明可由 Codex Runtime 直接承载。

### 4. 资源、工具和事件生命周期

#### 理论上如何桥接

完整 Runtime executor 应当：

1. 从 frozen Snapshot 获取 capability、Role Provider、MCP、skill、model route 和 typed resource ceiling；
2. 在调用 effectful tool 前经 Kernel/Host 做 authority 和 reservation；
3. 将 tool started/result、effect started/succeeded/failed/uncertain 变成 canonical SessionEvent；
4. 将 Runtime stream、assistant message 和 turn terminal 映射为同一个 operation；
5. 在 cancel、crash、delete 时关闭 turn、release resource lease，并清理全部 process descendants；
6. 从 Session event/compaction 重新构造历史，而不是读取一个不受 Host 约束的平行 transcript。

#### 当前实际缺口

- Sidecar 的 `RuntimeCreateParams` 只有描述性 capability/resource 输入，没有 Kernel handler 或 resource owner；
- `native_action/start` 只是当前自定义的 effect-start ACK gate，不是完整工具执行协议；
- Host 当前 `commit_native_action_start` 构造的 invocation input 是 `null`，并只记录 `effect/started`，不能代替带真实 arguments 的完整 capability dispatch；
- Runtime bridge 的 model request `tools` 为空；
- Runtime bridge 只处理 text output，遇到 function-call、native item 或 provider round 会以 unsupported feature 失败；
- Host bridge 不传完整历史，也没有 previous response/item 恢复；
- 官方 upstream 的 dynamic tool seam 是 `item/tool/call`，当前生产 client 没有把它接成 Kernel Role dispatch；
- 官方 upstream 的取消终态是 `turn/interrupt` 后等待 `turn/completed(status=interrupted)`，当前 stable client 使用自定义 `cancel`；
- 官方 upstream 没有 `runtime/session/dispose`，而当前合同依赖自定义 dispose ACK；
- Nomi 已有的 MCP manager、Browser lane、SSH lease、knowledge sink 和 artifact lifecycle 尚未由 Codex Runtime 统一承载。

## 兼容性矩阵

| 领域 | 重构目标 | 当前实现 | 评审结论 |
| --- | --- | --- | --- |
| Codex source | Codex engine/source 进入 NomiFun 构建图并成为核心 executor | 只有 `../codex` source lock、vendor metadata 和外部 executable 解析 | **不满足** |
| Runtime process | 可被 Host 隔离、限时、清理和校验 | `CodexRuntimeSupervisor` + `ManagedRuntimeProcess` 已有相应骨架 | 部分可用，仍依赖外部 artifact |
| Transport | 使用可验证的本机 Runtime transport | 当前 client 使用自定义 JSONL envelope；upstream spike 使用 initialize/thread/turn surface | 协议未闭合 |
| AgentSession | 一个 Session identity、durable events、turn receipt 和 delete lifecycle | `AgentPlatform`、`AgentSessionStore`、receipt/query 已有连接 | Host 侧连接成立 |
| Snapshot | frozen capability/model/resource execution closure | Snapshot 编译、digest 和 runtime profile 校验已有 | 合同可用，executor 消费不完整 |
| Capability/Role | 通过 Kernel exact dispatch，使用 Role Provider lock | Host `invoke_capability` 可以走 Kernel；Sidecar 未接完整 handler dispatch | 部分兼容 |
| Provider/Model | 复用多模型、connection、credential 和 route 管理 | Host Broker 可解析 exact route；Sidecar 不直接消费完整 binding | 部分兼容 |
| Coding model loop | Codex engine 或明确的单一模型 executor 完整处理 Coding turn | Host bridge 只执行无状态 text Responses 请求 | **不满足** |
| Tools | 文件、终端、VCS、MCP、Review、Test 等真实工具 loop | bridge `tools` 为空；custom native action 不完整 | **不满足** |
| History | Session event/compaction 能恢复完整模型可见历史 | bridge `previous_response_id=None`，只传当前文本和有限 instructions | **不满足** |
| Event lifecycle | Runtime、tool、effect、message、turn 共用 canonical operation | Host 可写部分 Runtime event，但 upstream event/tool mapping 未闭合 | 部分兼容 |
| Cancel | durable cancel 与 Runtime interrupt 的最终事实一致 | Host 有 cancel admission；当前 Runtime 仍是 custom cancel，缺 upstream terminal proof | 协议不兼容 |
| Dispose/delete | Runtime dispose、资源释放、Session tombstone 一致 | 外部进程树清理骨架存在；官方 thread/delete 与 custom dispose 语义未统一 | 部分兼容 |
| Remote | 当前 Nomi-core Remote 复用 canonical Session，并完成 ready turn chain；未来 FreshV4/Codex host 另行验证 | NomiCore 路径已有 provenance、deadline、fail-closed，产品 ready E2E 仍待真实 Provider/token；旧 FreshV4/Codex ready chain 才依赖 exact binary/live model | 当前 Nomi-core open work；不是 Codex Sidecar 阻断 |
| AgentExecution | attempt/participant/DAG 使用 canonical AgentSession | typed facade 仍包旧 Conversation/Nomi runtime | **不满足** |
| IDMM | 观察和介入绑定 canonical Session operation | 仍使用旧 Conversation summary/scope/stream；备用 sidecar 另有实现 | **不满足** |
| Nomi replacement | 未来 Codex 正式接替后才评估 Nomi 删除 | 当前 Nomi factory、ConversationService 和 runtime registry 是预期产品内核 | **后续阶段，不是当前缺陷** |
| 证据 | 真实 source、binary、provider、工具和平台 evidence | 主要是 fixture、adapter、broker 或 fail-closed evidence | 不能宣称完成 |

## 当前实现的硬不兼容点

以下不是“再补一个测试即可消失”的小缺口，而是当前实现与用户预期的执行模型之间的硬边界。

### A. 外置进程和代码归属

1. **源码归属不兼容**：`nomifun-codex-runtime` 没有 Codex engine source；它只实现 NomiFun 侧 client/supervisor。
2. **执行归属不兼容**：实际启动的是外部 artifact，Codex state 不在 NomiFun Kernel、AgentSessionStore 或 Nomi engine 的同一进程状态里。
3. **依赖图不兼容**：`nomifun-codex-runtime` 不依赖 `nomifun-ai-agent`、`nomifun-agent-kernel`、Provider repository 或 `nomifun-conversation`，所以它不可能单独执行这些领域能力。
4. **Nomi 替换未发生**：`AppServices` 仍构造 Nomi factory 和 `InMemoryAgentRuntimeRegistry`，旧消费者仍能通过 Conversation-backed adapter 运行。

### B. 协议、权限和模型调用边界

5. **协议 envelope 不兼容**：当前 Client/contract 允许 `runtime/hello`、`create`、`start_turn`、`cancel`、`runtime/session/dispose` 等自定义方法；upstream spike 记录的 surface 是 `initialize`、`thread/*`、`turn/*`，且没有这些自定义 RPC。
6. **adapter 未形成生产闭环**：`PinnedAppServerAdapter` 可以纯函数地把 stable method 映射到 upstream method，但当前 `CodexRuntimeClient` 仍按自定义 stable method 编码；静态引用没有证明 adapter 已被 production Client/Supervisor 使用。
7. **权限合同依赖自定义字段**：当前 adapter 强制写入 `approvalPolicy=never`、`sandbox` 或 `sandboxPolicy`，并拒绝调用方覆盖；这套字段组合与官方协议的真实版本和语义尚未由交付 binary 证明。
8. **模型所有者分裂**：Sidecar 接受 `StartTurn` 后，Host bridge 才调用 `ChatModelBroker`。Sidecar 不是完整模型 loop，Host 也不是只提供一个被 Codex engine 调用的 typed model port。
9. **凭据边界不等价于模型内嵌**：一次性 inherited handle 可以避免 Provider API key 进入 Sidecar，但也意味着外部 Runtime 没有复用 NomiFun 的完整 Provider connection/credential lifecycle。

### C. 工具回调和 Capability dispatch

10. **工具表为空**：当前 bridge 的 `ResponsesBridgeRequest.tools` 是空集合，不能证明 Coding surface 已经进入真实模型请求。
11. **custom native action 不是完整工具调用**：`RuntimeIngressPort` 只接受 `native_action/start`，Host 当前以 `null` input 做 identity/effect-start 校验并返回 ACK，不能代替带真实参数的 Role Provider dispatch。
12. **upstream tool seam 未接入**：官方 pinned source 的 dynamic callback 是 `item/tool/call`，当前生产 Client 没有将其路由到 `KernelRegistry` 或 `AgentSessionCommandPort`。
13. **资源 owner 不在 Sidecar**：Sidecar 收到 typed resource binding 描述，但没有 NomiFun 的 Browser/Computer/SSH/Knowledge/MCP owner、lease 或 repository；描述字段不能产生执行能力。

### D. 历史、事件和取消

14. **历史输入不兼容**：bridge 只传一条当前用户文本和有限 context instructions，不传完整 Session history、tool result、provider round 或 native response item。
15. **事件来源分裂**：Sidecar Runtime event、Host Broker message event、Nomi `AgentStreamEvent` 和 canonical `SessionEvent` 不是同一套完整事件模型。
16. **取消终态不兼容**：当前 stable `cancel` 与官方 `turn/interrupt` 不同；官方需要等待 `turn/completed(status=interrupted)`，而当前 Host 主要依赖 custom response 和 Session-side cancellation。
17. **dispose 语义不兼容**：当前合同期待 `runtime/session/dispose` ACK；upstream spike 记录官方使用 stdin EOF、connection close 和 `thread/delete`，没有该 Runtime dispose RPC。
18. **恢复身份不完整**：Session 使用 `AgentSessionId`，upstream 使用 thread/turn identity，当前没有已证明的 durable AgentSession-to-thread mapping、resume 和 fork 完整实现。

### E. 数据和生产依赖图

19. **部分消费者仍停留在 Conversation-backed 迁移边界**：Cron、Requirement/AutoWork、Channel、IDMM 和 AgentExecution 的生产适配器仍使用 `ConversationService`、旧 DTO、旧 runtime registry 或旧 turn scope；这是当前 Nomi-core 重构的 open work，不是改用 Codex Sidecar 的理由。
20. **Fresh-v4 schema 覆盖不完整**：canonical AgentSession schema 没有替代所有 automation、execution、channel 和 IDMM 旧表/旧事务边界；只添加 typed delegator 不会完成迁移。
21. **Gateway/Composition 仍有旧图**：transitional `AppServices -> build_module_states -> GatewayDeps` 仍手工组合大量具体服务。当前 NomiCoreApplication 保留这条原有 Nomi 组合以维持产品可运行；它仍需按真实消费者逐步收缩，FreshV4Application 不应被当作默认替代路径。
22. **测试证据不兼容**：fixture、synthetic observation、纯 adapter test 或直接 Broker smoke 都不能证明外部 Codex binary 已经成为核心 Coding runtime。

## 分期建议

### 本期：回到 `nomifun-ai-agent` / `nomifun-conversation` 的 Nomi 内核

本期应停止继续扩大 Codex app-server 的测试适配和外部 Sidecar 合同，把既有重构目标落在当前真实可执行的 Nomi 内核上。

建议实施顺序：

1. **保留 canonical AgentSession、Kernel、Role、Provider、Snapshot 和 Remote 合同**。这些是重构的有价值基础，不需要因为 Codex 方向止损而撤销。
2. **建立一个 Nomi-backed canonical Runtime adapter**。它只把 `AgentSession` 的 frozen Snapshot、operation identity、resource binding 和 active generation 投影给现有 `NomiAgentManager`，不新增第二个 Conversation identity，也不绕开 `AgentRuntimeRegistry` 的生命周期保护。
3. **把真实消费者逐项迁移到 canonical Session ports**：
   - Cron / Schedule；
   - Requirement / AutoWork；
   - Channel；
   - AgentExecution；
   - IDMM；
   - Companion 相关执行入口。
4. **在迁移中保留 Nomi 已有的完整能力**：Provider/Model resolver、历史、stream、工具注册、MCP、Knowledge、Browser/Computer、SSH、artifact、steer、cancel、failover 和 session persistence。
5. **收缩 AppServices、GatewayDeps 和旧 factory 组合**。迁移后的消费者只能依赖声明过的 typed port；旧 Conversation-backed adapter 只保留到明确的迁移窗口，不能继续增长。
6. **把 durable receipt 作为唯一 turn 终态**。不要从文本、idle、`Finish` 或 Runtime process 存活状态推断成功。
7. **以 Nomi 真实产品路径做 Chat/Coding 验收**。Provider live smoke 可以使用隔离的已授权配置，但它必须经过真实 AgentSession、Nomi runtime、Kernel tool dispatch 和 Session receipt，而不是只调用 Broker。
8. **本期不以 Codex Sidecar 为启动前置**。缺少 Codex binary、upstream 适配或 live credential 不应继续阻塞 Nomi 内核重构交付。

本期的完成定义应是：

```text
AgentSession
  -> frozen Snapshot
  -> Nomi-backed runtime
  -> exact Provider/Model route
  -> Kernel/Role/Capability tools
  -> canonical events/projections/receipt
  -> cancel/delete/recovery
```

这一链路真实可运行并由定向测试和人工产品验收证明后，才可以关闭本期对应重构项。它不需要宣称 Codex-native。

### 二期：重新评估 Codex-native

只有在本期 Nomi 路径稳定后，且产品仍然需要 Codex engine 的独特能力，才重新评估
二期。未来可以选择把实际 Codex engine 作为 NomiFun 的内部 Runtime implementation，
也可以通过保留的 `FreshV4Application` host boundary 重新接入外部 app-server；两者必须
明确命名和验收，不能把 external integration 称为“Codex-native 内核”。

候选实现形态只能二选一：

1. **源码进入当前 workspace 构建图**：将经过许可审查的 Codex crates/source vendored 或作为可复现 workspace dependency，并在 Rust 类型层直接实现 NomiFun Runtime port；
2. **明确的进程外产品**：如果因为许可、ABI、构建或安全原因必须保留进程边界，就应把产品名称和合同明确为 external Codex integration，而不能叫作“Codex-native 内核”。

若选择真正的 Codex-native，至少需要完成：

- Codex source、patch、license、notice、SBOM 和可复现构建进入仓库或受控依赖；
- Codex engine 不再由一个未接入的外部 executable 代表；
- Provider/model 请求只存在一个明确的所有者；
- Capability/Role/Resource 工具调用直接进入 Kernel authority；
- tool call、effect reservation、artifact 和 resource lease 映射到 canonical Session；
- history、compaction、resume、fork、steer、interrupt 和 terminal receipt 与 NomiFun 语义一致；
- AgentExecution 和 IDMM 只通过 canonical Session ports 使用它；
- `AppServices`、Gateway 和旧 Conversation graph 不再为 Codex 建立旁路；
- 真实的 source build、exact runtime、model、tools 和 platform evidence 一起通过。

### 三期：原生平台、发布和长期维护

三期才处理：

- Windows、macOS arm64/x64、Linux Desktop/Headless 的真实 native build；
- 签名、安装包、artifact digest、SBOM 和 license notice；
- 真实 process tree、sandbox/permission、TCC 和 OS resource 行为；
- upstream 升级、patch rebase、协议兼容和性能；
- Codex engine 与 Nomi engine 是否继续并存，或是否有明确的替换窗口。

三期不能用二期的 fixture、静态 source review 或 Host Broker 成功来替代。

## Codex-native 重新启动前置条件

下列条件全部满足前，不重新开启 Codex-native 的生产适配或 live 发布验证：

- [ ] **源码证据**：Codex engine source 或受控 workspace dependency 真正进入构建图；不能只有 `repository_alias`、commit 字符串或 source-lock。
- [ ] **构建证据**：能够从指定 source 在干净环境生成 Runtime，记录真实 build input、Cargo/Bazel 依赖、patch application 和 artifact digest。
- [ ] **所有权决策**：明确一轮 turn 只有一个模型执行所有者，消除“Sidecar 先 ACK、Host 再自行调用模型”的隐含双主。
- [ ] **Provider contract**：Provider、Model、Connection、credential lease、route revision 和 failover 都有一个 canonical resolver；Runtime 不读取 latest，也不复制另一套配置。
- [ ] **Capability contract**：真实 tool schema、Role Provider lock、typed resource binding 和 action arguments 能进入同一个 Kernel dispatch。
- [ ] **工具回调**：官方 `item/tool/call` 或等价内部 API 已经连接到 Host authority，并覆盖成功、失败、拒绝、重复和 unknown outcome。
- [ ] **历史与事件**：完整历史、tool result、compaction、resume、fork、assistant output、usage 和 terminal receipt 能从 canonical Session 重建。
- [ ] **取消与删除**：interrupt/cancel 的最终事实、dispose、thread/session 数据删除、resource release 和 descendant cleanup 有一致的生命周期合同。
- [ ] **AgentExecution/IDMM**：两者有 canonical Session adapter、exact operation scope、receipt 和 failover 语义；不再依赖旧 Conversation owner。
- [ ] **确定性验证**：fake transport 只用于覆盖协议分支，另有真实 exact binary 的一次有界 smoke；两种证据分开记录。
- [ ] **真实凭据验证**：使用隔离、可撤销、未写入日志或仓库的 model/provider credential，只验证明确授权的 turn/tool/cancel 场景。
- [ ] **平台证据**：目标平台有真实 Host、Runtime、package bytes、签名和 process/resource 结果；不能用其他平台结果替代。
- [ ] **用户 Go/No-Go**：在评估许可、构建维护成本、故障面和产品收益后，重新确认 Codex-native 仍值得承担。

现有 `SL-S2-10` upstream spike 只能满足协议调查输入，不能单独满足以上前置条件。

## 禁止误判的验收口径

### 1. 名称和文件不能替代实现事实

以下名称都不等于 Codex-native：

- `nomifun-codex-runtime` crate；
- `CodexRuntimeSupervisor`；
- `PinnedAppServerAdapter`；
- `CodingCodexContract`；
- `coding-runtime-feature-inventory`；
- `source-lock.json`；
- `patches/series.json`；
- `runtime-release-fixture.json`；
- `*.hello.json`；
- `nomifun-codex-runtime` 外部 executable。

它们最多说明合同、适配、监督或验证材料存在。

### 2. Fixture PASS 不能升级为产品 PASS

- Node app-server fixture 通过，只能证明 fixture 的 JSONL 和 Host 流程；
- synthetic `SessionObservation` 通过，只能证明校验器接受构造数据；
- adapter mapping test 通过，只能证明 method mapping 函数；
- fake provider transport 通过，只能证明 Broker bridge 的确定性分支；
- opening timeout/process-tree regression 通过，只能证明 Host cleanup 行为。

这些都不能证明真实 Codex source 已编入、真实 binary 可执行或完整 Coding 能工作。

### 3. Broker live PASS 也不能升级为 Sidecar PASS

直接调用 `ChatBrokerPort` 的 live smoke 只能验证 Provider route、credential 和模型 HTTP/stream 层。它没有经过：

```text
AgentSession
  -> Runtime Supervisor
  -> Codex app-server
  -> item/tool/call
  -> canonical tool/effect lifecycle
```

因此不能用它关闭 Codex Runtime、Coding、Remote ready-chain 或 AgentExecution。

### 4. `bun run dev` 不是 Coding 完成定义

`bun run dev` 正常启动只表示 Desktop Host、Vite/WebView、基础 API 和 UI bootstrap 可用。它不代表：

- Provider 已配置；
- Nomi 或 Codex model turn 已成功；
- Coding tools 已进入模型请求；
- 文件/终端/VCS/MCP 权限正确；
- cancel、history、artifact、IDMM 或 AgentExecution 已闭合。

### 5. Remote `open_failed` 不是 ready 通过

无 packaged Sidecar 时，Remote open 持久化 Session 后进入 `open_failed` 是 fail-closed 行为。它证明系统没有把 Sidecar 缺失伪装成成功，不证明 `open -> ready -> turn -> observe -> cancel` 已通过。

### 6. 本期 Nomi 验收与未来 Codex 验收必须分开

本期 Nomi 验收口径：

```text
真实 AgentSession
  -> NomiAgentManager / AgentEngine
  -> Host Provider/Model resolver
  -> Kernel capability/tool dispatch
  -> canonical event/projection/receipt
```

未来 Codex-native 验收口径：

```text
真实 Codex source/build
  -> 同一个 NomiFun AgentSession
  -> 同一个 Provider/Role/Capability authority
  -> 完整 history/tool/effect/cancel/delete lifecycle
  -> exact binary/platform evidence
```

两者之间不能用一个 adapter 名称、一个 fixture digest 或一次 Broker smoke 直接跳过。

### 7. 当前阶段不得宣称的结论

基于截至 2026-09-03 的实际代码，以下结论均不成立：

- “Codex 代码已经移植进 NomiFun 内核”；
- “Codex-native 已替换 Nomi”；
- “完整 Coding runtime 已完成”；
- “AgentExecution 和 IDMM 已经可以由 Codex Runtime 承载”；
- “Remote ready 产品链路已完成”；
- “C8、HP-1、C9 或 Stable 因这些 fixture/adapter/broker 测试而完成”。

当前最准确的工程表述是：

> NomiFun 已有一组 canonical AgentSession、Snapshot、Capability、Role、Provider 和
> Remote 合同；Web、Desktop 和 `nomicore` 当前统一选择 NomiCoreApplication/Nomi
> engine。仓库保留 FreshV4Application 与外部 Codex app-server 研究作为未来显式
> host boundary，但不提供运行中切换或 fallback。当前应先完成 Nomi-backed 重构，
> 再按前置条件重新评估 Codex-native 或 external Codex integration。

## 参考代码和审计记录

- `vendor/codex-runtime/README.md`
- `vendor/codex-runtime/source-lock.json`
- `vendor/codex-runtime/patches/series.json`
- `crates/backend/nomifun-codex-runtime/src/process.rs`
- `crates/backend/nomifun-codex-runtime/src/protocol.rs`
- `crates/backend/nomifun-codex-runtime/src/client.rs`
- `crates/backend/nomifun-codex-runtime/src/supervisor.rs`
- `crates/backend/nomifun-codex-runtime/src/adapter.rs`
- `crates/backend/nomifun-codex-runtime/src/release.rs`
- `crates/backend/nomifun-agent-platform/src/platform.rs`
- `crates/backend/nomifun-agent-platform/src/runtime_chat_bridge.rs`
- `crates/backend/nomifun-agent-platform/src/session_services.rs`
- `crates/backend/nomifun-agent-contracts/src/runtime.rs`
- `crates/backend/nomifun-agent-contracts/src/preset.rs`
- `crates/backend/nomifun-agent-contracts/src/session.rs`
- `crates/backend/nomifun-app/src/bootstrap/runtime_artifact.rs`
- `crates/backend/nomifun-app/src/bootstrap/canonical_host.rs`
- `crates/backend/nomifun-app/src/router/remote_runtime.rs`
- `crates/backend/nomifun-app/src/router/remote_rest.rs`
- `crates/backend/nomifun-app/src/router/state.rs`
- `crates/backend/nomifun-app/src/services.rs`
- `crates/backend/nomifun-ai-agent/src/factory/nomi.rs`
- `crates/backend/nomifun-ai-agent/src/factory/provider_config.rs`
- `crates/backend/nomifun-ai-agent/src/runtime_handle.rs`
- `crates/backend/nomifun-ai-agent/src/runtime_registry.rs`
- `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs`
- `crates/backend/nomifun-conversation/src/service.rs`
- `crates/backend/nomifun-conversation/src/runtime_state.rs`
- `crates/backend/nomifun-agent-execution/src/attempt_runner.rs`
- `crates/backend/nomifun-idmm/src/probe.rs`
- `docs/specs/2026-08-28-agent-capability-platform-v2/SIDECAR-UPSTREAM-SPIKE.zh.md`
- `docs/specs/2026-08-28-agent-capability-platform-v2/05-system-capability-replacement-foundation.zh.md`
- `docs/specs/2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md`
