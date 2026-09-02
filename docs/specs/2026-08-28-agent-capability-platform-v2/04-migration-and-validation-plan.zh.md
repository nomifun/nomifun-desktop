# 实施、迁移、清理与验证计划

> **经 2026-09-02 修订**
>
> 架构与范围权威：
> [05-system-capability-replacement-foundation.zh.md](./05-system-capability-replacement-foundation.zh.md)
>
> 当前任务、状态、Owner、依赖与验证记录：
> [GLOBAL-CLOSURE-TODO.zh.md](./GLOBAL-CLOSURE-TODO.zh.md)
>
> 本文只定义迁移方法、依赖边界、并发纪律和验证策略，不记录实时
> `open/blocked/closed` 状态，不复制 GLOBAL TODO，也不作为第二状态台账。

## 1. 修订目的

本次修订保留旧计划中仍然正确的架构方向：

- 统一 Capability、AgentSession、Runtime 和 Plugin 主链，不长期维护两套权威；
- fresh-v4 clean start，不读取、转换或双写旧业务数据；
- 用可独立验收的渐进切片迁移真实消费者，并在同一切片删除对应旧入口；
- 先稳定 canonical Compiler、Snapshot、Materializer 和 Session 基础，再迁移具体业务；
- Browser/Computer 先建立可替换 Role/Provider 接缝，再接第一方实现；
- 并行工作必须有互斥写集，中央合同和组合入口保持单一集成 Owner；
- 开发期优先定向验证，候选与发布阶段才运行必要的宽验证；
- 测试遇到环境或 harness 障碍时记录一次完整失败，转人工或等待输入，不盲目重试。

同时删除旧计划中已经被 05 否定的证明型工程：

- 不再使用固定的长阶段编号链或固定团队编组驱动实施；
- 不以固定 Capability、模板、action-bearing 数量或旧 inventory 总数判断完成；
- 不要求全仓历史字符串、文档和 fixture 达到统一的全量归零；
- 不建设多层候选证明、全平台整批复验编排、长期交接包或证明对象之间的 digest 镜像；
- 不建设按 Session 固定路由的长期双运行或影子流量平台；
- 不为所有 Effect、Replay 和故障排列建立通用矩阵；
- 不要求所有设计平台都阻塞首个 Stable。

本文的实施台账阶段采用 05 的一期 `S0-S5`。05 中的 `S6 Stable` 是 S5 之后对同一
已验证 RC 的发布提升动作，不在本文新增开发阶段。Browser/Computer 可替换能力在 `S3`
内部采用 05 的 `P1-R0` 至 `P1-R3`，它们是架构切片，不是另一套项目状态。

## 2. 核心迁移纪律

### 2.1 一条生产主链

每项一期能力最终只能有一条生产调用链：

```text
Product / Remote / Automation
            |
            v
canonical AgentSession + frozen Snapshot
            |
            v
Capability / Role dispatch
            |
            v
exact owner or Provider
```

Gateway 可以保留为 transport facade，但不能拥有业务 Registry、Preset、Session、
Effect 或 Provider 选择事实。业务能力不能继续进入 legacy Conversation、Factory、
`GatewayDeps` 或 `AppServices` 手工组合图。

架构原因：如果新入口只包裹旧服务，或者 Runtime 再回调旧 façade，迁移表面完成后仍会
保留第二个事实源和第二条执行路径，后续 Browser/Computer Provider、Remote 和自动化
都会再次重构。

### 2.2 Canonical source 只有一份

- Preview、Save 和 Test 共用一个纯函数 Compiler；
- Session Open 读取已保存 Snapshot，只做当前执行兼容检查，不重新编译另一份结果；
- Manifest 是声明事实源，Registration 从真实 handler/service exports 派生；
- Event Log 保存语义事实，Projection 只保存可重建的当前视图；
- Release fixture 只验证 schema，真实制品 digest 只进入 release lock/result。

发现同一事实需要在第二个 DTO、表、digest、coordinator 或手写清单中长期同步时，应停止
实现并回到 05 重新收缩。

### 2.3 渐进切片必须同改同删

迁移单位是一个可独立验收的垂直切片，而不是一个大领域批次、一个 crate 或一批未来 DTO。
每个切片在开工前至少明确：

1. canonical owner；
2. 真实产品或后台消费者；
3. 需要修改的互斥写集；
4. 前置合同和数据依赖；
5. 最小成功路径、失败路径与清理路径；
6. 本切片要删除的旧 route、wiring、fallback、测试和依赖。

同一切片必须完成：

- canonical contract 或 port；
- 真实 owner；
- 全部直接消费者切换；
- 必要 UI/API/资源绑定；
- 代表性定向测试；
- 对应旧生产入口、兼容分支和无消费者依赖删除。

不允许把旧入口删除留给未指定的“后续清债”。但这里要求的是本切片生产路径不再可达旧
实现，不是让每个切片扫描并分类全仓所有历史字符串。

### 2.4 切片施工与退出模板

切片进入施工前，必须已经具备：

- 已接受的 canonical contract 或 port，不在业务切片中临时发明第二套字段；
- 一个真实 owner、repository/handler 和至少一个真实消费者；
- 明确的数据所有权、事务/idempotency 边界以及 cancel/dispose 责任；
- 全部直接消费者和对应旧入口的有限清单；
- 能在当前机器执行的最小自动验证，以及自动验证受阻时的人工验收边界。

建议按以下顺序完成一个切片：

1. 先落最小 contract、owner 和持久化/资源边界；
2. 通过 `PluginRegistration`、typed port 或 canonical dispatcher 接入统一主链；
3. 切换一个真实 UI、API、后台或 Runtime 消费者，证明不是 metadata-only 实现；
4. 切换其余直接消费者，不让新旧 owner 同时接收生产请求；
5. 验证代表性的成功、typed failure、cancel/dispose，以及适用的外部 unknown-result
   no-retry；
6. 删除旧 route、DTO、Factory/Manager wiring、fallback、只验证旧行为的测试和无消费者依赖；
7. 形成可审查的 checkpoint commit，并只在 GLOBAL TODO 更新状态、阻塞和后续任务。

切片只有同时满足下列条件才可以关闭：

- 真实消费者从产品入口到 owner 完整执行，不能只注册或返回占位成功；
- owner、数据写入、外部副作用和资源清理责任唯一；
- 全部已登记直接消费者已切换；
- 对应旧生产入口从本切片相关 production roots 不可达，且不存在 alias、fallback 或双写；
- 自动或人工证据覆盖本切片声明的成功、失败和清理路径；
- 没有把必要删除、中央接线或用户可见缺陷转成无 Owner 的 follow-up。

### 2.5 不用兼容层换取表面进度

一期 fresh-v4 尚未 Stable，应直接修正 canonical schema、fixture 和调用链。默认禁止：

- alias endpoint、双读双写、compatibility view；
- deprecated façade 或只被 feature flag 隐藏的旧主链；
- Provider 缺失时回退第一方具体实现；
- Session 运行中重新选择 latest Provider；
- 为旧开发数据增加 migration 或 converter；
- 为通过旧测试而保留第二套对象、ID 或状态机。

真正仍有用户价值的旧实现可以作为新 owner 内部实现保留。例如 Browser Hub、
ComputerTool 和进程清理可以位于第一方 Provider 后面；不能继续由业务消费者直接持有。

### 2.6 先保护真实功能，再普通回退错误复杂度

在切片尚未被下游采用、没有发生不可逆数据边界或 C9 删除前，错误实现优先使用普通
revert 或小步删除后重做。回退前必须检查真实消费者，不能因为附加设计过重而删除已经
可用的用户功能。

禁止 `reset --hard`、force-push、共享历史重写以及清理不属于当前写集的用户改动。

## 3. Clean Start

Clean start 同时约束代码施工起点和 fresh-v4 数据起点。

### 3.1 代码施工起点

每个本地或远程任务开始时：

1. 记录明确的 base commit 和分支；
2. 检查 `git status --short`，保留并识别已有用户改动；
3. 声明本任务写集和中央文件接入需求；
4. 先运行能证明基线可工作的最小检查；
5. 不把 dirty worktree 当作无法开发的理由，但 dirty 结果只能作为诊断；
6. release attestation、候选制品和跨机交接必须来自 clean commit。

阶段性 checkpoint 应是可审查的普通 commit，包含一个完整切片或一个可编译的基础边界。
跨机协作通过普通 push/fetch/merge 传递，不依赖本机绝对路径、临时压缩包或未推送 SHA。

### 3.2 Fresh-v4 数据起点

fresh-v4 的目标不是兼容迁移，而是建立唯一的新数据事实源：

1. 在初始化前停止应用、Sidecar 和后台任务，并取得数据根独占权；
2. 在 cutover rename 或 clean-install mkdir 之前，先在 canonical root 的 parent 独占写入
   immutable bootstrap marker；marker 只保存规范化相对 basename、目标 generation 和
   canonical schema digest，不保存绝对路径、旧内容或 mutable stage；
3. 已存在 legacy canonical root 时，只允许同父目录、同文件系统的 whole-root 原子改名；
   clean install 则要求 canonical root 不存在，并在 marker durable 后才创建目录；
4. 禁止逐文件读取、转换、copy fallback、ID mapping、冲突合并或双写；
5. rename 或 fresh marker 成功后，在 canonical path 创建空 v4 root，只运行当前
   migrations、`schema_metadata`、bundled materialization 和 canonical seed；
6. 只有 ready marker、data generation、migration/seed/projection version 与 canonical
   digest 一致后，初始化才完成并删除 parent marker；
7. crash 或初始化失败时，只根据 immutable marker、parent 下精确路径、ready 和
   `schema_metadata` 判断恢复动作；只清理或重试未 ready 的新 root，不读取、修改或删除
   已归档 legacy root；
8. archive 不进入 Runtime、UI、API、CLI、watcher、backup 或恢复产品；
9. 用户在新 root 中重新配置 credential、Revision 和资源绑定。

具体模板和 Capability 集合由当前 canonical inventory 与产品合同生成，不由本文固定数量。

架构原因：只要启动链仍能读取 legacy root，所有后续 Compiler、Snapshot、Session 和
Provider 合同都会被迫承担兼容语义，clean cut 就会退化为长期双系统。

## 4. 一期阶段与依赖

以下是静态迁移边界，不表示当前完成度。实时状态只查看 GLOBAL TODO。

### S0：止损发布

目标：

- 发布 05 的新边界；
- 停止扩张旧 Gate、发布证明、通用 Effect 和平台矩阵系统；
- 让所有施工机器从同一权威文档重新领取任务。

退出依据：

- 不再由旧 inventory 数量和旧台账驱动实现；
- 当前状态只有 GLOBAL TODO 一份；
- 远程任务不再使用过期 Prompt。

### S1：Revert/Keep 审计

目标：

- 对没有真实 owner、repository 或消费者的批量合同做普通回退；
- 对已有真实用户价值的实现做 forward-simplify；
- 删除“因为已经写了所以必须继续兼容”的隐性约束。

架构原因：在错误抽象上继续接 owner，会把未来真实场景锁进未验证的 DTO 和状态机。

### S2：P0 与基础收缩

目标：

- 修复不可闭合的 source SHA/fixture release digest；
- 修复 Remote revoke 跨 Response Body 持锁；
- 简化 Session delete/dispose；
- 收缩 Event/Projection 与 Effect 策略；
- 合并 canonical Compiler，缩小 Snapshot 和 Registration；
- 以真实 upstream spike 冻结最小 Sidecar 协议。

S2 是后续工作的共同前置。Compiler、Snapshot 和 Registration 未稳定前，不应把更多具体
owner 接入中央组合图；Sidecar 协议未由 upstream spike 证明前，不应继续扩大 Host patch。

### S3：Role Seam 与核心 Owner

目标：

- 按 `P1-R0` 至 `P1-R3` 完成 Browser/Computer 可替换主链；
- 让 Chat、Coding、Workspace/File、Process、VCS、Knowledge、MCP、SSH、automation、
  Remote 和 Codex-derived Runtime 进入 canonical AgentSession 主链；
- 删除新 v4/Codex、Knowledge、Gateway、stdio 和 automation 的具体实现旁路。

依赖原因：

- 核心 owner 依赖 S2 的小 Effect、单 Compiler 和 Session authority；
- Browser/Computer 具体实现依赖先完成 exact Role contract、Resolver 和 Dispatcher；
- Remote 的完整产品链依赖真实 Runtime，而不仅是 transport/auth；
- automation 必须复用 canonical Session command/query，不能再构造 Conversation/Nomi runtime。

### S4：产品 UI

目标：

- 普通 Agent 编辑器只显示用户可理解的产品语言；
- Save/Test 自动调用内部 Preview；
- Workspace、Knowledge 和 Connector 使用 picker；
- 从模板创建、修改保存、选择资源试用、在新 Session 继续四条流程可验收；
- `bun run dev` 启动后 Desktop 不崩溃，用户不需要填写 UUID、operation 或 raw JSON。

S4 可以在 S2 DTO 稳定后并行开发界面骨架，但完整退出依赖 S3 的真实 owner 和 Runtime。

### S5：三平台、C9 与 RC

目标：

1. Windows Desktop x64 完成核心候选和完整用户闭环；
2. 同一候选在 macOS Desktop arm64 与 Linux Desktop x64 完成原生 critical smoke；
3. 候选冻结后执行一次性 C9 bounded shutdown 与 Nomi 物理删除；
4. 对同一 Nomi-free RC 在三个首发平台完成最终验证；
5. Stable 原样提升已验证 RC bytes。

macOS x64、Linux Headless x64 和非核心业务全覆盖保留后续交付入口，但不阻塞首个 Stable。

### 4.1 关键依赖图

```text
S0 -> S1 -> S2
               |
               +-> canonical Compiler
               +-> small Snapshot / Registration
               +-> Session Event / Effect
               +-> Sidecar upstream spike
                         |
                         v
                        S3
                          |
                          +-> minimal Runtime / Session lifecycle
                          |             |
                          |             +-> core owners / MCP / SSH
                          |             +-> automation / Remote
                          |
                          +-> P1-R0 -> P1-R1 -> P1-R2 -> P1-R3
                                        |
                                        +-> Browser / Computer / Knowledge
                          |
                          v
                        S4
                         |
                         v
                       S5
```

同一阶段内可以并行，但不能绕过箭头建立临时直连或兼容 façade。

### 4.2 工作流进入条件

并行准备不等于可以提前接入生产。下游只有在上游的最小合同和退出条件成立后才能合流：

| 上游边界 | 允许进入的下游 | 进入条件 |
|---|---|---|
| Canonical Compiler / Snapshot | Editor Save/Test、Session Open | Preview/Save/Test 同结果；Session 不二次编译 |
| AgentSession command/query + Runtime lifecycle | Chat、Coding、automation、Remote | `open/turn/cancel/dispose` 可真实执行并有终止路径 |
| Role contract + Provider Resolver | Browser/Computer first-party owner、Knowledge render | exact Provider lock 生效；缺失时 typed failure；无 fallback |
| Plugin/owner slice | UI/API/background direct consumers | owner、repository/handler 和资源清理已存在，不是占位 registration |
| Windows clean candidate | macOS/Linux native smoke | source 与制品身份冻结，已列出目标平台待验证点 |
| 三平台 pre-delete confidence | C9 | shutdown/dispose 可在有限时间完成，生产与制品不再需要 Nomi |
| C9 clean commit | Nomi-free RC / Stable | 三平台验证同一 RC bytes，Stable 不重新构建 |

如果下游只能通过临时 adapter、第二份 DTO、旧 Factory 或 fallback 才能开始，应继续在独立
写集中准备，而不是提前接入中央组合图。

## 5. S2 基础收缩实施边界

### 5.1 Canonical Compiler

唯一 Compiler 接收 Revision、当前 materialized inventory、资源和模型输入，输出：

- resolved Snapshot；
- authority/allowlist；
- diagnostics；
- deterministic digest。

Preview、Save 和 Test 只能调用它。Control Plane 只映射 diagnostics，不复制 closure、
profile 或 digest 算法。Session Open 不重新编译，只读取保存结果并做结构兼容检查。

最小验证：

- 同一输入经 Preview、Save、Test 产生相同 Snapshot 内容和 digest；
- 输入变化只影响实际执行闭包；
- 缺依赖、资源或 Provider 时返回同一 typed diagnostic；
- 不存在第二个可生产 Snapshot 的入口。

### 5.2 Snapshot 与 Registration

Snapshot 只冻结实际执行所需的 Capability、Provider、Tool schema、Model Route、资源和
Runtime features。未选择的全局 inventory、文档 digest 和无关模板不参与 Session 兼容。

Registration 从 Manifest 和真实 exports 派生，保留 namespace、schema、typed dependency、
duplicate/cycle 与 cleanup；不再手写第二份 operations/declared metadata 做自我证明。

### 5.3 Event、Projection 与 Effect

- SessionEvent 是语义事实；
- Projection 不复制完整 Event Log；
- 正常完成只持久化最终 assistant message；
- 中断最多保留一份 bounded partial；
- 本地状态变更使用事务、CAS 或原子文件；
- 外部结果可能未知的操作在 dispatch 前保存最小 reservation，unknown 时不自动 retry；
- reconcile 归 owning domain，不建立全局 EffectCoordinator。

验证只覆盖发生变化的具体语义，不建立所有 Tool 和所有故障点的通用排列组合。

### 5.4 Sidecar Upstream Spike

先验证 official app-server 已有 initialize/version、thread/turn、cancel、event、
Host-managed Tool 和进程关闭能力。一个 Runtime binding 可以先独占一个受管进程。

只有 upstream 无法提供必要的调用前接缝时，才提出一个窄 patch。不得先假设历史自定义
RPC 必须存在，再要求 Host、fixture 和发布合同围绕它扩张。

## 6. S3 的 Browser/Computer 子阶段

Browser/Computer 必须通过一个完整架构切片交付，不能先直连第一方实现再等待二期重构。

### P1-R0：合同冻结

冻结唯一 canonical 定义：

- `ExecutionRoleId` 与 versioned Role Contract；
- Package/Mount Provider identity；
- source-neutral `RoleProviderContribution`；
- required/optional member；
- typed action、Context 和 Resource exports；
- installation binding、Revision override 和 Snapshot lock；
- typed missing/unavailable/incompatible errors。

`browser.*` 和 `computer.*` 保持 canonical façade；Provider 不能抢占相同 Capability ID。

退出条件：canonical Rust、schema generator、target inventory、Registration 和文档只保留
一套字段定义；Browser/Computer required/optional member、Context/Resource export 与 typed
error 可以从同一来源生成或校验。

架构原因：如果这些位置各自维护一套字段，后续 Resolver 和二期 loader 会再次分叉。

### P1-R1：Registry、Resolver 与 Snapshot

- Materializer 接收同一 Provider contribution；
- Registry generation 发布 flat `role_provider_index`；
- Resolution 顺序固定为 Revision override、installation default、missing typed failure；
- 先选择 exact Provider，再按其 member platform/resource 约束编译；
- exact contract、Package、Mount、contribution 和资源进入 Snapshot lock；
- non-Agent operation 使用同一个 Resolver 和可传递 exact lock；
- 运行中不重新选择 latest，也不因失败自动 fallback。

退出条件：first-party 与 alternate fixture 能生成不同但自洽的 Snapshot，消费者代码不变；
non-Agent operation 也能携带同一种 exact lock。

### P1-R2：Browser/Computer Clean Cut

- bundled Browser/Computer 通过普通 materialization 发布 façade 与 first-party Provider；
- Kernel 第一次路由直接选择 frozen Provider Mount；
- Tool、Context 和 Resource 都读取同一个 exact lock；
- Browser 保留 owner/lane/close/cancel/process cleanup；
- Computer 按 exact target resource 串行单次 physical action，并校验 observation generation；
- Knowledge `browser.render_content` 使用 non-Agent operation exact lock；
- Gateway、业务 Factory、Service bag 和 stdio bridge 不再直接构造具体 backend；
- v4/Codex/automation 不复用 Nomi-only Browser slot；
- Provider 缺失或不兼容时明确失败，不回退第一方。

人类 Browser 管理、登录、Surface 和生命周期模块可以作为具名 owning surface 直接使用底层
Hub；它们不是 Agent/automation Browser Use 的旁路。例外必须精确到用途，不能扩大到
Knowledge、Gateway 或业务消费者。

退出条件：v4/Codex Agent、automation、Knowledge、Gateway 和 stdio bridge 不再引用具体
Browser/Computer backend；只有 first-party Provider implementation 与具名的人类 Browser
owning surface 可以引用 Browser backend，Computer backend 只由 first-party Provider 引用。
精确的 Nomi-only wiring 可以等待 C9，但不得增长、接入新消费者或成为 fallback。

### P1-R3：一期 Gate 收口

只运行与本切片直接相关的验证：

- Role contract、Materializer、Resolver、Compiler 和 Dispatcher；
- first-party 与 alternate fixture parity；
- Snapshot resume、unavailable 和 no-fallback；
- Browser owner/lane/process cleanup；
- Computer target serialization、generation 和 platform-unavailable；
- Knowledge、Gateway、stdio 与代表性 automation consumer；
- Windows 候选中的 Browser/Computer 核心流程；
- macOS/Linux 在 S5 执行原生 critical smoke。

实现退出条件：上述定向验证通过，变更进入 clean Windows candidate source，旧结果不能冒充
本次变更的验证。发布退出条件在 S5 关闭：实际 Host、Sidecar 和 Package digest 进入
release lock，同一候选在三个首发平台留下 platform result。

P1-R3 退出后才能宣称 Browser/Computer 可替换接缝完成；它不代表二期 Plugin/MCP Provider
安装、选择和 UI 已交付。

## 7. 核心业务切片

S3 中除 Browser/Computer 外的业务迁移按真实用户闭环排序，不按旧领域批次数量机械搬运。

推荐顺序：

1. 按 S2 upstream spike 结论实现最小 Codex-derived Runtime、Model Bridge 和
   `open/turn/cancel/dispose`；
2. Chat、Workspace/File、Process、VCS、Knowledge search/read；
3. Coding 的 read/write/patch/shell/diff/commit；
4. 一个真实 MCP Tool；
5. 精简 SSH read/write/exec/sudo；
6. 一个真实 scheduled/automation AgentSession；
7. Remote `open -> turn -> observe -> cancel`。

顺序不是状态表。某些互斥写集可以并行，但合流后必须保持：

- 一个 AgentSession command/query authority；
- 一个 Capability/Role dispatch；
- 一个 Compiler/Snapshot 事实源；
- 一个 owner 对外部 Effect 负责；
- 一个明确的 cancel/dispose/process-tree cleanup 路径。

没有成熟产品数据模型、真实 repository 或真实消费者的非核心能力，不应先批量冻结合同。
它们可以明确 unavailable，也可以留到后续阶段，但不能返回 metadata-only success。

### 7.1 关键切片退出条件

| 切片 | 必须满足的退出条件 |
|---|---|
| Runtime / Session | 真实 Sidecar/Runtime 能 open、turn、cancel、dispose；启动失败和 cancel timeout 最终进入 terminal 状态并清理 descendant process |
| Chat / Coding / 核心 owner | 真实产品入口通过 frozen Snapshot 调用 canonical owner；成功、typed failure 和 cleanup 可验证；不经过 legacy Conversation/Factory |
| MCP / SSH | 至少一个真实 Tool/host binding 执行；schema、资源、Secret、timeout/cancel 语义明确；写或命令结果未知时不自动重试 |
| Automation | 使用用户显式保存的 AgentBinding/Revision/资源创建普通 AgentSession；不推断 hidden default/latest，不构造 Nomi runtime |
| Remote | `open` 返回 durable `agent_session_id` 和可观察 cursor；`opening` 只能在独立 deadline 内转为 `ready` 或 terminal `open-failed`，不能永久停留，也不能接收普通 Turn |

Remote 的 Runtime ACK mismatch、Sidecar crash 或 deadline 到期必须写入单一 terminal failure，
dispose 已创建的 Runtime handle，并保证 Tool/Effect 未开始。相同 Idempotency-Key 只返回同一
opening/ready/failed 结果，不创建第二 Session；token revoke 提交后的旧 token 新 admission
稳定失败，提交前已接受的有限操作正常收敛。

## 8. 并发与所有权

并发规模由可独立写集决定，不设固定 Agent 数量或固定长期工作流数量。

### 8.1 单写者区域

以下区域在同一时间只允许一个集成 Owner 修改：

- canonical contracts、Compiler 和 Snapshot；
- Kernel Materializer、Registry 与 RoleDispatcher；
- fresh-v4 schema/migration registry；
- central composition root、`AppServices`/`GatewayDeps` 拆除入口；
- workspace manifest、`Cargo.lock`、根 Gate 和 release scripts；
- GLOBAL TODO 与跨机任务分配。

其他任务通过窄 patch 或独立 commit 请求接入，不并发编辑这些中央文件。

### 8.2 可并行区域

满足前置合同后，以下工作通常可以使用互斥写集并行：

| 区域 | 并发边界 | 合流规则 |
|---|---|---|
| Session Projection 与 Effect | 同一 Session crate 内由一个 writer 串行 | 先 Projection，后 Effect；分别定向测试 |
| SSH owner | 只修改 SSH crates 和专用 tests | 不改 Compiler、Kernel、App composition 或 lockfile |
| Sidecar upstream spike | 只写 spike 记录、trace 和专用验证 | 结论被接受前不改生产 Runtime |
| Browser first-party Provider | 只改 Browser implementation/adapter | 公共 Role/Dispatcher 由中央 Owner 接入 |
| Computer first-party Provider | 只改 Computer implementation/adapter | 与 Browser 分离，公共 arbiter 由中央 Owner 接入 |
| UI 产品化 | DTO/Compiler contract 稳定后修改 UI 与定向 tests | 不自行发明后端兼容字段 |
| 原生平台 smoke | 候选冻结后在真实目标机执行 | 修复按普通 commit 回主机合流 |

### 8.3 多机启用门槛

独立机器只有在能承担多个互不冲突的开发、修复或验证任务时才值得启动。只有一个很小任务
时由主机正常排期，避免交接成本高于并行收益。

当前机器分配、Batch 内容和状态只查看 GLOBAL TODO 及其链接的当前 Prompt。跨机回传至少
包含 base SHA、commit SHA、changed paths、实际验证命令、未运行项和主机接线说明。

### 8.4 合流纪律

- 每个 writer 只提交自己的写集；
- 合流前检查 `git status --short`、`git diff --check` 和 staged paths；
- 同一中央文件的变更由集成 Owner 串行处理；
- 不让多个 Agent 同时运行会争用相同 DB、固定端口或 Cargo build directory 的重测试；
- 合并冲突按当前 canonical contract 解决，不为两个分支同时存活增加 adapter。

### 8.5 多机分支与回传协议

多机协作以 Git 中可检出的 clean commit/ref 为边界，不建立长期 handoff 包：

1. 主机先推送 clean base，并在 GLOBAL TODO 或其当前任务说明中声明多个任务、互斥写集、
   依赖和中央文件接入点；
2. 远端机器 fetch 后核对 base SHA，再创建自己的普通分支/worktree；未核对成功不得施工；
3. 远端只提交已分配写集中的完整切片，不盲目 merge 主分支，也不 rebase/force-push
   共享历史；基线变化由集成 Owner 明确决定继续、普通 merge 更新或重新领取任务；
4. 每个可独立审查的切片形成普通 checkpoint commit；未完成实验与本地大型日志不进入提交；
5. 回传时 push 分支，并报告 base/head、commit 列表、changed paths、实际验证、未运行项、
   blocker、人工步骤和中央接线需求；
6. 主机 fetch 后验证 ancestry、工作树、diff 和写集，再以普通 merge 合流；中央文件和冲突
   只由集成 Owner 修改；
7. 合流后只重跑受影响检查；发现共享合同或制品变化时，再按 §10.5 扩大验证。

远端分支没有新提交时可以 fast-forward 到主线；已有远端提交时不得为了“同步”覆盖或重写
它们。任何机器都不能依赖另一台机器的绝对路径、脏工作树、未推送 SHA 或临时压缩包。

## 9. 验证策略

### 9.1 最小充分验证

验证强度随变更风险扩大：

| 层级 | 适用时点 | 最小验证 |
|---|---|---|
| 文档/配置 | 纯文档或局部配置 | `git diff --check`、链接/术语检查 |
| Edit loop | 单 crate 或单 UI 组件 | format、受影响 crate `check`、精确 test/filter |
| Slice | 一个真实 owner/consumer 闭环 | 定向 unit/integration + 一个代表性成功/失败/清理路径 |
| UI flow | 产品流程变化 | 相关 UI tests、typecheck；必要时 `bun run dev` 人工验收 |
| Windows candidate | S5 候选冻结 | 核心用户闭环、package/install/fresh/launch、关键 lifecycle |
| Native smoke | macOS arm64 / Linux Desktop x64 | 真实 OS/CPU 的 package/install/launch/critical capability/dispose |
| Nomi-free RC | C9 后 | 三平台同一 RC bytes 的 package/install/fresh/critical E2E/lifecycle |

全仓 broad check 只在跨模块主要合流、Windows 候选和最终 RC 需要时运行。多个 Agent 不应为了
各自局部改动重复运行相同宽测试。

### 9.2 行为专用测试，而不是通用矩阵

只有具体合同需要时才增加 replay 或 fault 测试，例如：

- Projection 能从 Event 重建；
- 外部 unknown Effect 不自动 retry；
- Remote revoke 不跨 Response Body 挂起；
- bootstrap crash 不读取 legacy archive；
- cancel/dispose 能清理受管进程树；
- Browser/Computer 缺 Provider 时明确失败且不 fallback。

不要求每个 Tool、每个状态和每个故障点做统一全组合。测试必须证明真实风险，而不是让
代码适配一个与产品无关的证明框架。

### 9.3 测试挂起与环境障碍

所有可能挂起的 E2E 必须有独立 deadline。遇到失败或超时后：

1. 保存第一次完整命令、base/head、日志和环境；
2. 判断是代码缺陷、harness 缺陷、外部权限/credential、网络还是目标平台缺失；
3. 没有代码、配置或环境变化时不重复运行相同命令；
4. 能定向缩小时只运行最小复现；
5. harness 不稳定但产品可人工验证时，提供人工步骤；
6. 需要用户输入时明确列出唯一缺失信息；
7. 无真实环境时记录“未运行”，不得构造 mock PASS 或 synthetic evidence。

障碍记录至少写清：

- clean/dirty 状态、base/head、命令、环境和第一次完整失败日志位置；
- 分类为代码缺陷、harness 缺陷、外部权限/credential、网络或目标平台缺失；
- 它阻塞的是哪个切片、自动化证据、平台声明或发布条件，而不是笼统写“全部阻塞”；
- 已完成的最小复现、可人工关闭的范围、仍不能关闭的范围；
- 重新执行所需的具体代码、配置、网络、credential、硬件或用户输入变化。

人工验收记录至少包含：

- 前置条件；
- 操作步骤；
- 预期结果；
- 需要保留的截图、console 或 backend 日志；
- 实际结果和失败点。

人工验收可以关闭 UI/产品流程证据，但不能代替真实 macOS/Linux 原生制品验证，也不能把
未构建的 Sidecar 或缺失 credential 记为通过。

禁止为绕过障碍而放宽断言、吞掉错误、加入 test-only 生产分支、无依据增加 sleep/deadline、
把 mock 结果记为成功，或重复运行同一长测试直到偶然通过。代码缺陷阻塞对应切片；纯
harness 障碍在人工证据足够时只保留自动化债务；缺少 credential、网络或目标硬件只阻塞
对应能力/平台声明，不阻塞无关切片。三个首发平台的真实原生验证仍是 S5 发布条件。

### 9.4 两类生产清理检查

开发切片只检查：

```text
本切片的新生产入口不能再到达对应旧实现
```

发布阶段只检查：

```text
最终 package / binary / config / process 不包含 Nomi Runtime 或 fallback
```

文档、历史测试文本和 schema fixture 不进入复杂 allowed/deferred/unclassified 分类。必要的
删除清单可以用于人工审查，但不建设长期全仓规则引擎，也不要求无关历史内容统一归零。

### 9.5 最小发布证据

发布追溯只保留：

- Gate 运行时读取的 clean source commit；
- 打包后生成的真实 `release-lock` 及 Host/Sidecar/Package digest；
- 每个目标平台的 source、target、实际 suite、结果和日志引用；
- 实际未运行项及原因。

Pre-run fixture 不包含自身 commit SHA，不把假 digest 当真实制品。文档或状态文件变化不会
使字节未变化的制品自动失效；Host、Sidecar、Package、Runtime protocol 或目标平台代码变化
时，只重跑受影响检查。最终 Stable 仍必须是三个首发平台验证过的同一 RC bytes。

## 10. S5 平台与发布收口

### 10.1 Windows 核心候选

Windows Desktop x64 是主开发和完整核心闭环平台。候选至少验证：

- package、install、fresh start、launch；
- Chat、Coding、File、Process、VCS、Knowledge；
- MCP、Browser、Computer、SSH/automation、Remote；
- cancel、crash、Session delete 和 process-tree cleanup；
- Desktop 可由 `bun run dev` 正常启动并完成四条产品流程；
- 无 P0、数据损坏或 Secret 泄漏。

### 10.2 macOS 与 Linux 原生 Smoke

Windows 候选冻结后，macOS Desktop arm64 与 Linux Desktop x64 可以在两台真实机器并行：

- 构建并打包当前候选；
- install、fresh、launch；
- 执行目标平台 critical capability；
- 验证 Sidecar 与 dispose；
- 记录平台明确 available/unavailable 的能力。

cross-compile、WSL、VM、emulation 或 Rosetta 可以作为开发预检，但不能代替目标平台结果。
平台发现的共享修复通过普通 commit 回主机；只重跑被实际制品或协议变化影响的检查，不启动
全平台全功能排列组合。

### 10.3 一次性 C9 Clean Cut

三个首发平台对删除前候选建立足够信心后，执行一次：

```text
停止 Nomi 新 admission
-> 取消内部 Nomi 工作
-> bounded application/runtime shutdown
-> kill descendant process tree
-> 外部 unknown Effect 保持 uncertain 且不自动 retry
-> 确认生产 route/process/release artifact 不再依赖 Nomi
-> 物理删除 Nomi runtime/factory/adapter/package
```

这里不建设在线 drain 平台、祖先 deadline 传播、跨会话 durable 迁移协调器或多维
outstanding 证明账本。删除窗口只需要真实 shutdown/dispose 结果和生产/release 无 Nomi 入口。

### 10.4 Nomi-free RC 与 Stable

C9 后从 clean commit 构建 Nomi-free RC。Windows、macOS arm64 和 Linux Desktop x64 对同一
RC bytes 执行 package/install/fresh/critical E2E/lifecycle。Stable 只提升这些已验证 bytes，
不重新构建另一个制品。

如果 RC 修复改变共享代码或制品，按影响范围重跑必要平台检查；不要求无关平台重做全部
功能，也不恢复 Nomi 作为 fallback。

### 10.5 候选变化与重验范围

候选与 RC 由 clean source commit 和实际 artifact identity 共同确定。任何修复只要改变二者
之一，就产生新候选，不能把旧日志或其他 SHA 的结果拼到新候选上。

重验按变化面决定：

- 只改文档或不进入制品的开发脚本：执行文档/脚本定向检查，不重跑原生制品；
- 只改目标平台 adapter/package：重跑该目标平台的 build/package/install/launch/critical
  smoke，并运行共享 contract 的最小检查；
- 改共享 Host、Sidecar、Runtime protocol、Package materialization、数据 schema 或核心
  Session lifecycle：三个首发平台重跑受影响的 critical smoke，Windows 补充对应核心闭环；
- 只改某个产品流程：重跑该流程的成功、失败和清理路径；只有制品或共享 ABI 变化才扩大到
  其他平台。

修复应先在主机合流并形成新的 clean checkpoint，再分发目标平台验证。不得按固定次数全量
复验，也不得因一个局部修复恢复旧的全平台全功能笛卡尔积。

## 11. 回滚与失败语义

### 11.1 不可逆边界表

| 边界 | 允许的恢复动作 | 明确禁止 |
|---|---|---|
| 只有代码变更，切片未被消费者采用，bootstrap marker 未写入 | 普通 revert 完整隔离切片，或删除错误 abstraction 后按更小合同重做 | 部分恢复旧协议、覆盖用户未提交工作 |
| Immutable marker 已 durable，但 root 尚未 ready | 先运行同一 bootstrap recovery，使文件系统回到确定状态，再决定代码修复 | 手工跳过/删除 marker、逐文件搬运、用 Git 回滚替代数据恢复 |
| Legacy root 已 whole-root rename，或 v4 已写入用户数据 | 在 canonical v4 内 forward-fix；未 ready 的新 root 只按 marker 规则清理/重试 | rename-back、读取 archive 恢复、legacy import、data downgrade |
| 直接消费者已切换且旧入口已删除 | 保持 canonical owner，forward-fix 当前切片 | 恢复 alias、fallback、双写或旧 Factory/Manager wiring |
| C9 前候选失败 | 停止该候选，合入修复后构建新的 clean candidate | 为赶进度提前 C9，或恢复已关闭的 legacy slice |
| C9 后、Stable 前 | 丢弃失败 RC，从 Nomi-free forward-fix commit 构建新 RC | revert C9、重新打包 Nomi、混用 pre-delete evidence |
| Stable 发布后 | halt rollout；选择兼容 frozen Snapshot/数据的 previous same-v4 Host/Sidecar、精确 Preset/model route，或发布新 RC | Nomi/pre-v4 binary、archive 恢复、数据降级、重建未经验证的“同版本”制品 |

### 11.2 共同失败语义

- 回滚或发布停止都必须保留用户数据、用户未提交工作和已发生的真实领域事实；
- 外部结果为 unknown/uncertain 时不因 rollback、restart 或 replay 自动重试，也不自动补偿；
- previous same-v4 artifact 只有通过当前数据、Runtime protocol 和 frozen Snapshot 兼容检查后
  才能重新部署；
- 候选失败后保留真实日志和 blocker，但新的候选必须使用自己的 source/artifact/结果；
- 数据损坏、Secret 泄漏、无法终止的受管进程、核心用户流程不可用或任一首发平台 RC
  critical smoke 失败时，必须停止 promotion，而不是降低门槛。

### 11.3 必须停下来重新设计的信号

出现以下任一情况时，当前任务不得继续堆代码：

- 需要第二份相同事实；
- 需要新的全局 digest 或证明对象；
- 需要新的通用 coordinator 或状态机；
- 需要所有平台与功能的全组合重验才能说明局部代码正确；
- 需要用户编辑内部 ID、digest、operation 或 raw JSON；
- 需要 alias、fallback 或双写才能让测试通过；
- 没有真实 owner 和消费者，却要先冻结大批 DTO；
- 同一失败在环境未变化时只能靠不断重试；
- 需要修改未授权中央文件或与其他 writer 争用同一写集。

处理方式是回到 05 核对产品价值、所有权和最小合同，而不是在旧设计上增加例外。

## 12. 静态退出定义

这些定义用于判断阶段边界，不在本文记录是否已经满足。

### S0 退出

- 05 成为一期权威；
- 旧证明系统不再驱动施工；
- 状态只由 GLOBAL TODO 维护。

### S1 退出

- 批量无消费者合同已有 keep/revert 结论；
- 保留项都有真实用户价值和明确简化边界。

### S2 退出

- P0 Gate、Remote hang 和 delete/dispose 问题关闭；
- 单 Compiler、小 Snapshot、简化 Registration、Event/Projection 和三类 Effect 生效；
- Sidecar 最小协议来自真实 upstream spike。

### S3 退出

- `P1-R0` 至 `P1-R3` 完成；
- Browser/Computer first-party 与 alternate fixture 走同一 Role seam；
- 核心 owner、MCP、SSH、automation、Remote 和 Codex-derived Runtime 可真实执行；
- Remote `opening` 在 deadline 内只收敛到 `ready` 或 terminal `open-failed`；
- 新 v4/Codex 生产消费者没有具体实现旁路或 legacy fallback。

### S4 退出

- `bun run dev` 可正常启动；
- 四条真实用户流程可人工或自动验收；
- 普通用户不接触内部标识和 raw JSON。

### S5 退出

- Windows、macOS arm64、Linux Desktop 的同一 Nomi-free RC 通过；
- C9 已物理删除 Nomi；
- release lock/result 可追溯；
- Stable 原样提升同一制品。

未达到某项退出条件时，具体任务、阻塞原因、Owner 和下一步只更新 GLOBAL TODO，不回写本文。
