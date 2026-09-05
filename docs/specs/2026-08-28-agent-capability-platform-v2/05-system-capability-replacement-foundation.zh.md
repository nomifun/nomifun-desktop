# NomiFun 一期止损修订：简化重构与可替换系统能力基础

> 状态：**USER-CONFIRMED PHASE 1 STOP-LOSS DIRECTIVE / AgentPreset AP-0～AP-7 前置 TODO / 尚未完成代码实施**
>
> 发布日期：2026-09-02
>
> 审计基线：`f6e05d617e09eb71ebb11fababde46bb65039651`
>
> 适用范围：正在进行的 Agent Capability Platform v2 一期重构，包括主集成分支、机器 2 SSH lane、后续 Browser/Computer、Sidecar 与原生平台验证任务。
>
> 阶段门禁：二期 `06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md` 可以继续作为设计文档修订，但在本文新增的 AgentPreset 前置 TODO（AP-0～AP-7）完成并通过门禁前，不得进入 Plugin/MiniApp 代码实施；本文不把 06 的设计状态视为一期代码合同。

## 0. 本文的权威与执行方式

用户已经明确授权立即止损：一期已经确认或实现的细分方向，如果继续实施的成本明显高于产品价值，可以普通回滚、删除并按更小合同重新设计；不得因为“已经写了很多代码”继续追加复杂度。

本文不是在旧方案外面再加一层兼容规则，而是一期当前的定向修订：

- Thin Kernel、统一 Plugin/Capability 主链、单一 AgentSession、Codex-derived Runtime 和 clean v4 等总目标继续有效；
- 本文明确列出的 Gate、Evidence、生命周期、Compiler、Snapshot、Effect、文件边界、产品 UI 与平台矩阵改用本文的新策略；文末新增的 AgentPreset 平台级能力建设（§15）是进入二期 06 前必须完成的后续 TODO；
- 01～04 与 `DECISIONS` 保留为核心设计依据，并按本文删除或改写其中已经判定错误的条款；不得因局部设计被止损而整体删除这些文档。旧 `IMPLEMENTATION-STATUS`、旧 `START-PROMPT` 和过期 handoff 只保留在 Git 历史；当前状态只由 `GLOBAL-CLOSURE-TODO.zh.md` 记录；
- `GLOBAL-CLOSURE-TODO` 的 84 个工作包不再是一期必须逐个关闭的阻断清单；只把其中仍属于本文最小交付的项目迁入新的收口清单；
- 已生成的 Manifest、fixture、digest 或结构测试不能因为自身存在而阻止删除；证明系统不具有高于产品系统的优先级；
- 实施只采用普通 commit/revert/merge，不使用 reset、force-push 或历史重写；回滚前先检查下游消费，保留真实用户功能和无争议的基础正确性。

本文中的伪代码和字段名是设计输入。实施后仍以 canonical Rust、SQL、API schema 和行为测试为机器事实；不得复制第二套文档结构并要求逐字段长期同步。

施工机器的最短阅读路径：先读第一部分 §0、§1、§3、§10、§12、§14，立即停止错误方向；随后完整读取第二部分，按保留的一期 Role/Provider 合同实施 Browser/Computer；准备进入二期前还必须完整读取文末 §15。产品与架构复核应阅读全文。

## 1. 所有正在施工的机器先做什么

读取本文后，先保存当前工作，不丢弃未提交内容，然后按下列边界重新领取任务。

### 1.1 立即暂停

以下方向在完成本文对应简化前不得继续扩张：

1. C8/C10 Evidence、四元 cohort tuple、handoff、recheck、digest envelope 和 residual 分类系统；
2. D-027 在线 canary drain、祖先 deadline、durable handoff 和多维 exact-zero proof；
3. 为所有本地写入统一增加 `started/succeeded/failed/uncertain/reconciled`、receipt、outbox 和 replay matrix；
4. 为防御同权限恶意本地进程而继续扩张逐组件 no-follow、系统目录别名和 TOCTOU 证明；
5. 没有 production repository、真实消费者和产品入口的 Wave 3/4 DTO、receipt、reconcile、migration 和 fault matrix；
6. 要求用户直接编辑 Capability ID、Revision、Snapshot、Digest、Resource ID、operations 或 canonical JSON 的 UI；
7. 读取 JSX/Rust 源码字符串并锁死组件、方法名、固定 Capability 数量的结构测试；
8. Codex fork 新 patch，直至 §3.4 的最小 Sidecar 协议重新确认；
9. Browser/Computer 具体 owner 的中央接线，直至 §7 的 Role seam 先落地；
10. 机器 2 旧 SSH Prompt 中“可证明原子覆盖、通用 uncertain receipt、中央 Effect journal 和旧 API 长期兼容”的扩张。

### 1.2 可以继续

以下工作仍有直接产品价值，可以保留或在新合同下继续：

- 真实 Chat/Coding、文件、进程、VCS、Knowledge read、SSH、MCP、Browser、Computer owner；
- 用户选择工作区、知识库和连接器的产品化 picker；
- Package/Capability ID、owner/resource binding、Secret 不泄漏和最小输入上限；
- Runtime cooperative stop、timeout 后 whole process-tree hard kill；
- 外部不可逆 Effect 的 idempotency identity 与 unknown-result no-retry；
- target crate 的普通 unit/integration tests；
- 不扩张旧合同的明确 bug 修复。

### 1.3 停止条件

如果一个任务必须新增以下任一内容才能继续，应先停止并回到本文重新判断：

- 第二份相同事实；
- 新状态机或新 coordinator；
- 新的全局 digest；
- 新的跨平台笛卡尔积；
- 用户无法理解的内部字段；
- 只为低概率恶意本机场景服务的大段代码；
- 仅用于证明另一个证明对象正确的 fixture。

## 2. 为什么必须现在止损

最新全局台账显示：

- 128 个 Capability 中有 81 个 action-bearing，但真实 action owner 只有 20 个，仍缺 61 个；
- Wave 3/4 当前 production owner 都是 0，却已经为 typed contract 与 Effect 状态新增约 8,300 行；
- 当前台账有 84 个工作包，其中 33 blocked、15 external；
- 最终 native evidence 仍是 0/5；
- 最近 8 个提交新增约 11,100 行，主要仍是合同、DTO、receipt 和 owner scaffolding；
- `gate-agent-v2.mjs` 已达到约 8,100 行，Rust validation contract、generator、macOS helper 和 9 份阶段 Manifest 又重复表达同一批事实；
- 完整 C8 已出现 25 分钟运行、单测试挂住 7.4 分钟，以及小修复导致全部 evidence 失效。

这表明主要瓶颈已经不是实现用户能力，而是满足一套过大的证明体系。继续按旧台账逐项补齐，会把一期拖成内部平台工程，并把同样复杂度传给二期 Plugin/MiniApp。

## 3. 必须立即修正的 P0 问题

### 3.1 删除无法闭合的 source SHA 自引用

当前 Generator 把固定历史 SHA 写进 Platform Manifest，而 Native Gate 要求该字段等于当前 clean HEAD。修改并提交 Manifest 后 HEAD 又变化，因此无法闭合。

新规则：

- pre-run input 不包含它自身所在 commit 的 SHA；
- Gate 运行时读取当前 clean commit；
- post-run result 记录 source commit 与实际 Artifact digest；
- 文档、状态和 Gate 自身变化不使字节完全相同的已验证 Artifact 自动失效。

### 3.2 Fixture 与真实 Release Manifest 物理分离

当前 Generator 用 `sha256("agent-v2-contract-fixture:<label>")` 覆盖 Host、Sidecar 和 Package digest，又让 Native Gate 要求真实制品等于这些 fixture digest。这不是可接受的 release contract。

立即改为：

```text
runtime-release-fixture.json
  只验证 schema，明确使用假 digest

release-lock.json
  打包后生成，只记录真实 Host/Sidecar/Package digest

platform-result.json
  记录目标平台、实际执行的 suite、结果和日志引用
```

Host binary 不嵌入自身完整文件 digest；以外部 release lock 对账。删除 JS、Rust、helper 和 generated envelope 中对同一 digest 的重复硬编码。

Post-run release lock/platform result 属于外部或发布附件证据，不写回它所证明的 candidate source commit；如需在 Git 归档，归档文件引用 immutable candidate SHA，不要求归档 commit 等于被测 commit。

### 3.3 修复 Remote token revoke 卡死

当前 Remote middleware 把 `RemoteRequestAdmissionPermit` 持有到 HTTP Response Body 被完整消费或释放；客户端只读 status 就执行 revoke 时，写锁可能永久等待。这已经导致集成测试挂起。

新规则：

- token validator 的原子 generation/hash 状态是请求认证线性化点；
- 验证成功的请求可以完成，之后验证的旧 token 立即失败；
- 删除跨 Response Body 生命周期的异步读锁；
- 不建设 grace、token→Session 索引或后台 revoke worker；
- 保留 installation token、同 owner continuation 和明确 `REMOTE_AUTH_REQUIRED`。

### 3.4 重开 Codex Sidecar 最小协议

当前计划依赖仓库外尚不存在的 `runtime/hello`、`native_action/start`、`runtime/session/dispose` patch source，导致真实 Sidecar 成为整个一期的外部硬阻塞。

立即停止继续围绕缺失 patch 扩大 Host contract，先验证最小方案：

1. 优先复用官方 Codex app-server 已有 initialize/version、thread/turn、cancel 和 event 协议；
2. 一个 Runtime binding 当前独占一个受管进程，正常结束可先关闭协议，再由 Host 终止整棵进程树；如果没有真实资源必须在进程内复用，不要求自定义 `session_dispose` ACK；
3. Host-managed Tool 的请求到达 Host 时，先写必要的外部 Effect reservation，再执行并返回结果，不再额外发送一次 `native_action/start`；
4. 只有 Codex-native file/shell action 确实无法用现有 upstream callback/Tool seam 获得最小调用前通知时，才保留一个窄 patch；
5. hello 只校验协议 major、build identity 和支持的必要 feature，不携带整个产品/平台合同镜像。

完成一个真实 upstream spike 后再确定是否保留浅 fork。不得因为已写 Host adapter 就假设三项自定义 RPC 都必须存在。

## 4. 生命周期与数据正确性的止损

### 4.1 D-024 保留产品语义，删除虚假证明

保留：

```text
live → deleting
→ 停止新写入
→ cooperative dispose
→ hard-kill descendant process tree
→ 幂等删除 Session 自有内容
→ minimal tombstone
```

删除：

- `ZeroOutstandingProof::verified()` 及其字符串计数表；
- 由调用者填写九个或十五个零来“证明”真实资源已经消失；
- 每类 handle 的发布级证明对象。

Runtime 使用真实 `RuntimeDisposeReport`；Session Store 只删除自己拥有的表。启动时发现 `deleting` 就重新执行幂等清理并完成 tombstone，不恢复复杂 Delete Operation 状态机。

### 4.2 D-027 从在线排空平台降为一次性 C9 shutdown

一期是尚未 Stable 的本地桌面重构，v4 又采用 fresh start。Internal Nomi canary 不需要服务器级零停机迁移。

新 C9 前置：

```text
停止 Nomi 新 admission
→ 取消全部内部 Nomi 工作
→ bounded application/runtime shutdown
→ kill descendant process tree
→ 对无法确认的真实外部 Effect 记 uncertain
→ 验证 Nomi process、binding、public route、release artifact 不再存在
→ 删除 Nomi
```

删除祖先 deadline 最小值、per-domain sticky canary、read-only shadow、durable Session handoff 和多维 outstanding ledger。禁止同一 Session 中途切 Runtime、禁止自动重放 Effect继续保留。

### 4.3 SessionEvent 保留，Projection 不再复制 Event Log

保留一套语义 SessionEvent、cursor 和最终用户/助手消息；删除 `message_projection` 内嵌完整 `events[]` 的重复存储。Projection 只保存 UI 当前需要的最终状态、文本、Tool 摘要和引用。

模型 token/delta 默认 transient：

- 正常完成只持久化最终 assistant message；
- 中断时最多保存一份 bounded partial；
- 不为每个 content part 重读、追加并重新序列化全部旧事件；
- Event Registry 只保留被 Store/Projector/Runtime 实际执行的字段。

### 4.4 Effect 生命周期只保留三种策略

```text
read_only
managed_effect
external_uncertain_effect
```

- 本地 DB、KV、文件和 VCS 使用事务、revision/CAS 或原子文件操作，记录一个最终 Tool result；
- 外部发送、远程命令、设备控制及其他结果可能未知的操作，dispatch 前持久化 reservation，unknown 时禁止自动 retry，由 owning domain reconcile；
- EffectClass 可以继续作为展示与路由 metadata，但不能让所有非读操作自动进入同一完整状态机；
- 删除 Wave 级 JSON/CAS Effect journal、固定 128 条记录和与 SessionEvent 重复的 receipt；
- 不建立全局 EffectCoordinator。

## 5. Compiler、Snapshot 与数据库的止损

### 5.1 只保留一个 canonical Compiler

当前 Control Plane 和 Kernel 各自计算 Snapshot，Session Open 又重新编译并做不完整 convergence 比较。立即合并为一个纯函数 Compiler：

```text
Preview ─┐
Save ────┼─> one canonical Compiler
Test ────┘          │
                    └─> Snapshot + authority + diagnostics

Session Open ─> 读取已保存 Snapshot + 当前执行兼容检查
```

Control Plane 只把 diagnostics 映射成产品 DTO，不再复制依赖解析、closure、profile 和 digest 算法。删除第二份 checkpoint/Snapshot compatibility 实现。

### 5.2 Snapshot 只锁定实际执行闭包

必须冻结：

- 实际选中的 Capability/Provider/Package contribution；
- 实际 Tool schema、Model Route 和 typed resource binding；
- 当前需要的 Runtime protocol/features；
- initial/on-demand 分组；
- Snapshot 自身 digest。

不再用以下全局事实决定旧 Session 是否可执行：

- 未选择的 Package/Capability；
- 整个 target inventory；
- 官方模板全集；
- 决策文档 digest；
- 与当前 Session 无关的全局 schema ledger。

兼容性只在 Runtime binding 建立、实际 Capability 激活或其执行实现变化时检查并缓存；不在每个普通 Turn 对完整 ceiling 重算。仍禁止静默换 Provider、修改旧 Snapshot或降级 Coding。

### 5.3 删除未执行的 CapabilitySelection 字段

首版只保留当前真实消费的：

- capability ref；
- action allowlist；
- resource binding refs。

`required`、`exposure`、destination constraints、context/tool budget override、未传入 Handler 的 config 等字段在有真实执行语义前删除。Initial/on-demand 由所在集合表达。

### 5.4 删除重复和无人读取的数据库投影

保留：

- `agent_presets`；
- immutable revision payload JSON；
- Snapshot envelope JSON；
- Agent/Remote binding；
- 真正需要独立索引的 model route；
- Session fact/event/projection 表。

删除或合并当前只写不读、与 JSON 重复的：

- snapshot capability projection；
- runtime profile projection；
- 未使用的 preset audit events；
- 没有产品查询者的 capability pack 与 preset 子表；
- 同时保存 content JSON 和包含同一 content 的 envelope JSON。

Fresh-v4 尚未 Stable，应直接修正 baseline 和 fixture，不为开发数据增加兼容 migration。未来正式升级只按 data generation、migration lineage/checksum 和 schema compatibility 判断，不要求完全相同的应用 build/决策 digest 才能打开用户数据。

### 5.5 简化 PluginRegistration

Manifest 是声明事实源，Registration builder 从实际 handlers/services 自动派生 registrar metadata。保留 Package/Mount namespace、config schema、Capability ID、typed service dependency、duplicate/cycle 检查与 cleanup；删除 Package 手写第二份 `allowed_operations/declared_*` 来证明自己和 Manifest 相同的工作。

## 6. 文件、SSH 与本地可信边界

### 6.1 文件系统只守基本正确性

必须保留：

- 用户明确选择的 Workspace/Knowledge root；
- canonicalize、root containment、拒绝 `..` 和明显越界；
- 文件类型、单文件和总量上限；
- 写入使用同目录临时文件 + rename；
- 删除和覆盖前明确目标；
- 错误不泄漏不必要的宿主路径。

首版不保证抵御同权限恶意本地进程在每个 syscall 之间替换 symlink/junction。已完成的 anchored Knowledge read 可保留真实功能，但应 forward 简化；不要继续为 macOS 系统目录别名、每个 component handle 和极端 TOCTOU 扩大专用平台层。`knowledge.write` 按基本 containment + atomic replace 实现，不再等待分布式式可证明 CAS/uncertain 协议。

### 6.2 机器 2 中的 SSH slice

旧单 SSH Prompt 已删除，机器 2 只在当前 Batch Prompt 能一次承载多项并行工作时启动。
其中 SSH writer 保留：

- 真实 SSH connection/host binding；
- read/write/exec/sudo 最小 typed command/outcome；
- 基本 path/payload/output/timeout 上限；
- exec 与 sudo 凭据分离；
- host-key changed 明确失败；
- cancel/timeout 后关闭或回收连接，不自动重复写/命令；
- Secret 不进入日志、参数和测试。

停止：

- 为所有 SFTP write 证明跨服务端绝对原子覆盖；
- 通用 `succeeded/failed/uncertain/reconciled` 平台；
- 为中央 Effect journal 预制复杂 receipt；
- 为即将删除的旧 API 设计长期兼容层；
- 无真实 SSH 环境时构造大规模模拟证明。

机器 2 的启用门槛、独占写集和回传格式以当前
`MACHINE-2-PHASE1-BATCH-A-START-PROMPT.zh.md` 为准；若只剩 SSH 单项，不启动独立机器。

## 7. Browser/Computer 可替换能力不得在止损中缩水

原 05 的 Browser/Computer Role Provider 设计是一期必须完成的主体，不属于本次要删除的过度设计。其完整产品目标、机器合同、全局旁路清理、实施顺序和一期/二期边界在本文第二部分原样保留，并升级为已确认的一期合同。

本次止损只调整它所依赖的外围底座：

- 先合并双 Compiler，再把 Role Provider 接入唯一 Compiler；
- 收窄 Snapshot 的全局无关 digest，但 exact Provider lock 必须保留；
- 简化 Gate/Evidence，但 first-party dogfood、alternate fixture 和全局无旁路验证必须保留；
- 简化 Effect 与文件防御，但 Browser/Computer 的资源生命周期、Computer target ordering 和 whole process cleanup 必须保留；
- 不用同 ID 抢占、built-in shortcut 或运行时 fallback 换取表面简单。

一期仍必须实现原 05 中已经定义的：

- Browser/Computer versioned Role Contract；
- source-neutral RoleProviderContribution；
- installation default binding 与 Agent Revision override 机器字段；
- ResolvedRoleProviderLock；
- Agent Snapshot 与 non-Agent operation exact binding；
- Tool、ContextContributor、ResourceProvider 所需的 typed runtime seam；
- Kernel 第一次路由直接选择 Provider Mount；
- Provider-specific platform/resource availability；
- Knowledge hidden render、Gateway 和 computer stdio 等旁路清理；
- 第一方 Provider 与 test alternate Provider 走同一 materializer/index/dispatch。

在本部分一期 Role/Provider seam 完成且 §15 的 AP-0～AP-7 门禁通过后，二期 06 才负责开放平台级 Node Plugin/MCP/MiniApp contribution、用户选择 UI、Chat Dev 和后续 CLI/Skill adapter；Plugin/MiniApp 的能力不能被设计成 Agent 专属，也不能把一期完整接缝推迟到二期。

## 8. 产品体验止损

后端 Revision、Snapshot、typed resource 和 digest 可以保留，但普通用户界面不再原样暴露这些概念。

Agent 工作台的编辑视图默认只展示：

- 名称与用途；
- 模型选择；
- 按用户任务分组的能力/能力包开关；
- 工作区、知识库和连接器 picker；
- 保存；
- 试用 Agent。

默认行为：

- Initial/on-demand 由模板和 Capability metadata 自动决定；只在开发者模式覆盖；
- binding ID、resource ID、operations、owner 和 typed parameters 由后台生成；
- Save/Test 自动执行内部 Preview；不要求用户先点 Preview；
- Test 只保留一个“试用 Agent”，打开普通真实 Session；
- Revision、Snapshot、digest、protocol、raw Event/JSON 放入默认折叠的“技术详情/导出诊断”；
- Snapshot 不兼容时显示“在新会话中继续”，后台执行显式 fork；
- 删除提示只说删除内容且无法恢复，不列 Projection、checkpoint 等内部表。

测试只覆盖主要用户流程：从模板创建、修改并保存、选择资源并试用、在新会话继续。删除读取源码字符串、固定组件存在、ASCII 分隔符和固定 `137` Capability 数量的结构测试。

## 9. 发布与测试策略止损

### 9.1 首批 release-blocking 平台

一期与二期统一优先级：

1. Windows Desktop x64；
2. macOS Desktop arm64；
3. Linux Desktop x64。

macOS x64 与 Linux Headless x64 保留设计兼容和后续交付入口，但不阻塞首个 Stable。未来实际宣称交付时再在真实原生环境关闭各自 Gate。

### 9.2 新收口链

```text
S0 STOP-LOSS
  发布本文、停止旧扩张、完成 revert/keep 审计

S1 FOUNDATION
  P0 Gate/Remote 修复 + 单 Compiler + 小 Snapshot/Schema/Event/Effect

S2 CORE FUNCTIONAL
  Windows 上完成 release-required 用户闭环和 Browser/Computer Role seam

S3 NATIVE SMOKE
  Windows x64 + macOS arm64 + Linux Desktop x64
  build/package/install/launch/critical capability/dispose

S4 C9 CLEAN CUT
  bounded shutdown + 删除 Nomi + release residual-zero

S5 FINAL RC
  三个平台对正式 RC 运行 package/install/fresh/critical E2E/lifecycle

S6 STABLE
  原样提升已验证 RC bytes
```

C8 不再在五个平台完整复制全部功能/fault；C10 不再重跑 C8 的所有内部合同测试。相同 Artifact digest 可以复用证据；只有真实产品 ABI、Runtime protocol、Package 或目标平台 Artifact 改变才使对应 cell stale。

### 9.3 两类 residual

- 开发期：`production_legacy_reachability = 0`，只检查新 Session/public route 是否还能进入旧主链；
- Release：`release_legacy_artifacts = 0`，检查最终 feature/package/binary/config/process 不含 Nomi。

Docs、tests、fixtures 和历史字符串不进入复杂 allowed/deferred/unclassified 分类。Deletion manifest 保留为人工审查清单，不为每个旧符号建设长期规则引擎。

### 9.4 测试执行

- dirty worktree 可以运行完整 verify，但结果只是诊断；
- 只有 release attestation 要求 clean commit 和真实 Artifact；
- Cargo 默认并行，只有共享 DB、固定端口和进程树测试单独串行；
- 每个可能挂起的 E2E 有自身 deadline；
- 全仓 broad test 只在主要合流和最终 RC 执行；
- Catalog 测试验证 ID 唯一、依赖闭合和关键角色可运行，不锁固定数量；
- 代表性真实 E2E 优先于 source-string、fixture-shape 和排列组合测试。

## 10. 现有提交的保留、重做与回滚

### 10.1 保留

| Commit/范围 | 处置 | 理由 |
|---|---|---|
| `3f835174` canonical Session services | 保留 | 单一 AgentPlatform/Session authority 是必要基础 |
| `280841b3` Knowledge picker | 保留 | 直接改善用户体验，避免手填内部 ID/path |
| `8aade375` VCS push owner | 保留真实 owner，forward 简化 journal/receipt | 已有用户功能和真实调用，不因附加复杂度删除主体 |
| Knowledge search/read | 保留真实功能，forward 简化 anchored FS | 已有生产消费者；不继续扩大极端本机攻击保证 |
| File/Process/VCS 现有真实 owner | 保留 | 属于 release-required 高频闭环 |

### 10.2 优先 ordinary revert 后重做

以下两个提交当前各自在单个 Wave 文件净增约 4,100 行，仍无 production repository/owner，后续提交也尚未形成真实消费者。实施者应先做一次下游依赖检查；若结论保持不变，按逆序普通 revert，再从真实用户场景重新添加最小 DTO：

1. `d1acccf6 feat(agent): type wave3 action contracts`；
2. `765d1953 feat(agent): type wave4 effect contracts`。

重做原则：一个实际 owner + 一个真实消费者 + 一个代表性 test 才引入合同；不先批量冻结 19/11 个 DTO、receipt 和 reconcile 状态等待未来填充。

### 10.3 不做的 Git 操作

- 不 reset、force-push 或改写远程历史；
- 不回滚包含真实用户功能的整批提交，只因其中部分设计复杂；
- 不通过兼容 alias 保留被删除合同；
- 不把尚未通过 §15 门禁的 06 代码加入任何一期 commit；06 的设计修订可以继续，但不能被误当成已授权实施；
- 不在机器 2 分支盲目 merge 主分支；主机发布本文后，由任务 owner 明确通知它更新基线或重新领取精简 lane。

## 11. Release-required 产品闭环

一期 Stable 不再以“所有 inventory 项都有 owner”为完成标准，而以真实用户闭环为准。

首批必须：

- `chat.minimal` 正常对话；
- `coding.codex` 完整核心 Coding：读写、patch、shell/process、diff/commit；
- Workspace/File/Process/VCS 高频能力；
- MCP 连接与一个真实 Tool 调用；
- Browser observe/navigate/act；
- Computer observe/input（平台可用时）；
- Knowledge 选择、search/read；
- 创建、继续、取消、删除 AgentSession；
- 一个真实 scheduled/automation Session；
- Remote open/turn/observe/cancel；
- Plugin `sample.echo` 证明第一方/未来第三方同链；
- Runtime crash/cancel/process-tree cleanup 代表路径。

不是首批阻断项：

- 没有成熟产品数据模型的 Wave 3 Creation/Workshop/Office/MiniApp Agent action；
- Knowledge autogen/embedding/rerank/write 的全部高级组合；
- 每个 Channel/Robot/Customer/Creative 场景的完整排列；
- 所有 optional Capability 的真实设备/live credential 验证；
- macOS x64、Linux Headless；
- 性能 benchmark、统计质量评测和长观察窗口。

非首批能力不注册进官方默认模板，或清晰显示“尚未提供”；不得用 metadata-only success，也不得阻止核心 Stable。

## 12. 实施顺序与退出条件

### S0：发布与暂停

- 提交并推送本文、经本文修订的 01～04/`DECISIONS`、README 和 GLOBAL TODO；
- 二期 06 继续保持“设计中、未授权实施”的状态，不得随一期核心代码发布；是否提交 06 设计不改变其实施门禁；
- 主机和机器 2 在新 commit 后重新读取本文；
- 旧 GLOBAL TODO/Machine Prompt 冲突部分停止执行。

### S1：Revert/keep 审计

- 检查 `d1acccf6`、`765d1953` 下游依赖；
- 无真实消费者则普通 revert；
- 为保留提交列出只需 forward 简化的代码；
- 不开始新 Domain DTO 批量冻结。

### S2：P0 与基础收缩

- 修复 source SHA/fixture digest Gate；
- 删除 Response Body auth fence；
- 用真实 dispose result 替换 ZeroOutstandingProof；
- 单 Compiler、选中闭包 Snapshot、简化 schema/projection/effect；
- 完成 Sidecar upstream spike并冻结最小协议。

### S3：Role seam 与核心 owner

- 先完成 Browser/Computer role index、exact lock 和 first-party dogfood；
- 再接 Browser、Computer、Knowledge hidden render；
- SSH/MCP/核心 automation 使用简化合同接入；
- 新 v4/Codex concrete bypass 为 0。

### S4：产品 UI

- 普通 Agent 工作台只保留产品语言和 picker；
- 技术 Inspector 默认隐藏；
- 四条真实用户流程通过；
- source-string structure tests 删除。

### S5：三平台收口与 C9

- Windows 完整核心闭环；
- macOS arm64/Linux Desktop package/launch/critical smoke；
- bounded shutdown 后删除 Nomi；
- 最终三平台 RC 验证与 same-bytes Stable。

## 13. 完成定义

### 13.1 止损完成

- 不可闭合 Gate 和 Remote hang 已修复；
- 两个 revert 候选已有明确 keep/revert 结果；
- 84 项旧 TODO 已缩减为 release-required 清单；
- 旧机器 Prompt 不再驱动新复杂度；
- 06 没有进入一期代码发布，且其实现仍被 §15 AP 门禁阻断。

### 13.2 一期功能完成

- §11 的核心用户闭环在 Windows 可用；
- Browser/Computer 经可替换 Role seam 调用，无 built-in shortcut；
- 单 Compiler/小 Snapshot/小 Effect 策略生效；
- Nomi 已从 production/release 主链删除；
- 非首批能力不会制造成功或阻塞核心交付。

### 13.3 内部 QA 可交付

- 三个首发平台使用真实候选 Artifact 完成 package/install/launch/critical smoke；
- Windows 完成代表性功能、失败和进程清理；
- 没有 P0 blocker、数据损坏或 Secret 泄漏；
- 当前 release lock 与 platform results 可追溯。

### 13.4 Stable

- 三个平台的正式 RC Gate 通过；
- Stable 原样提升同一 RC bytes；
- 不包含 Nomi Runtime/fallback；
- macOS x64/Linux Headless 未交付时在产品和发布说明中明确，不伪装支持。

## 14. 给正在重构机器的最终指令

1. 拉取包含本文的最新 `rf/agent-capability-platform-v2`；
2. 不再按照 84 项 GLOBAL TODO 顺序继续施工；
3. 保存当前 WIP，先执行 S1 Revert/keep 审计；
4. 立即处理 P0，再完成单 Compiler/小 Snapshot/Effect 分级；
5. Browser/Computer 先做 Role seam，再做具体 owner；
6. 机器 2 SSH lane 按 §6.2 缩减，旧 Prompt 不再是完成标准；
7. 不等待五格两轮 Gate，不为未交付平台生成 synthetic evidence；
8. 可以阅读和修订本地 06 设计，但不读取其作为已冻结代码合同，也不实现未通过 §15 门禁的 06；
9. 每个提交说明它删除了什么旧复杂度，不能只增加新 abstraction；
10. 遇到“继续兼容更快”与“普通 revert 后重做更干净”的选择时，优先后者，但必须先保护真实用户数据和已完成用户功能。

本文已经获得用户对止损方向和立即发布的明确授权。实现中的字段命名可以按 canonical source 收敛；任何需要重新扩大平台、状态机、权限、兼容或测试矩阵的变化，必须重新提出产品理由，不能自动恢复旧设计。

## 第二部分：系统能力可替换基础（原 05 完整合同，继续作为一期必做）

> 状态：**USER CONFIRMED / PHASE 1 REQUIRED**
>
> 保留原则：本部分完整保留原 05 的产品与技术内容，不因第一部分的止损审计而降级、删除或推迟。
>
> 优先级：第一部分负责纠正 Gate、生命周期、Effect、文件边界、平台矩阵和 UI 等过度设计；本部分负责 Browser/Computer 可替换能力。二者发生文字冲突时，能力范围与 Role/Provider 主链以本部分为准，发布/验证编排以第一部分为准。
>
> Canonical source：本部分 Rust/SQL 结构仍是字段级设计输入；落地后由唯一 canonical Rust/SQL/schema 实现承载，不维护第二份漂移结构。

### 1. 一期必须保证的产品结果

一期不需要让用户立即安装第三方 Browser/Computer Provider，但一期结束时必须保证二期不再重构系统主链即可交付以下体验：

1. NomiFun 把 Browser Use、Computer Use 理解为两个稳定的系统能力角色；
2. 第一方 Browser/Computer 只是这两个角色的默认实现；
3. 所有 Chat、Agent、Cron、AutoWork、Requirement、MiniApp 及其他业务消费者都只调用稳定角色，而不认识具体实现；
4. 二期加入 Plugin 或 MCP Provider 后，只需增加 Provider、选择与产品体验，不需要再次修改所有消费者；
5. 新 Session 使用当时解析出的精确 Provider，已有 Session 不因全局选择变化而漂移；
6. Provider 缺失或不兼容时明确失败，不静默换回第一方实现；
7. 用户未来可以看见实际执行的是哪个 Package、Mount、版本和 Artifact，而不是把第三方实现伪装成第一方实现。

Role Binding 只回答“由谁实现”，不授予 Browser/Computer 能力。Agent 是否能够调用某个 member，仍由现有 Revision/Snapshot Capability selection、allowlist 和 typed resource binding 决定；选择 Provider 不能扩大 capability ceiling。

一句话完成定义：

> 一期负责证明“系统以后可以换实现而不改主链”；二期负责让用户真的能够安装、选择、测试、切换和恢复 Plugin/MCP 实现。

### 2. 为什么这部分不能全部推迟到二期

#### 2.1 当前已经具备的基础

一期现有实现已经完成了若干正确基础：

- `browser.*` 与 `computer.*` 已拥有 canonical Capability ID；
- Browser、Computer/A11y 已作为 bundled Package 进入普通 `PluginRegistration`；
- Capability Materializer 和 Kernel Registry 已按 Capability ID 统一物化和调度；
- Agent Revision、Resolved Snapshot、Capability allowlist 与 typed resource binding 已形成唯一主链；
- Browser 已有 owner/lane 生命周期模型，Computer 已有单物理桌面和全局串行模型。

因此，本课题不是另起一套插件系统，而是把已经存在的 Capability 主链从“固定后端”改成“稳定 façade + 精确后端”。

#### 2.2 当前仍存在的直接耦合

| 当前事实 | 代码位置 | 对二期的影响 |
|---|---|---|
| Wave 2 Host 对 Browser、Computer 和 MCP 仍统一返回 unavailable | `crates/backend/nomifun-app/src/router/agent_wave2_host.rs` | Browser/Computer 的真实 canonical owner 尚未接通，现在仍有机会直接按最终结构完成 |
| legacy Nomi Factory 使用一次性 `BrowserLaneClientProviderSlot` 晚绑定具体 Provider | `crates/backend/nomifun-ai-agent/src/factory/browser_lane.rs` | 新 v4/Codex 主链不能复用；该 Nomi-only Slot 按 D-020 精确留到 C9 后整体删除，不值得再做过渡改造 |
| Computer Gateway 直接持有并调用 `ComputerRegistry` | `crates/backend/nomifun-gateway/src/caps_computer.rs`、`computer_registry.rs` | 二期 Provider 选择只能覆盖部分路径，Gateway 和后台消费者仍可能绕过 |
| `mcp-computer-stdio` 自行构造 `ComputerTool` | `crates/backend/nomifun-app/src/commands/computer_stdio.rs` | Codex/ACP 可以完全绕过 Snapshot Provider lock，是比 Gateway 更直接的第二执行主链 |
| Knowledge URL 渲染通过 `BrowserFetcher` 直接调用 `BrowserSessionHub` 的 `navigate/rendered_html` | `crates/backend/nomifun-ai-agent/src/browser_fetcher.rs`、`nomifun-app/src/services.rs` | 只替换 Agent Tool 会留下“Chat 用第三方、Knowledge 仍启动第一方 Chromium”的隐藏旁路 |
| `browser.*` / `computer.*` 的 `CapabilityManifest.supported_platforms` 当前直接承载第一方实现的平台范围 | `crates/backend/nomifun-agent-domain-wave2/src/lib.rs` | 未来远程 MCP/Plugin Provider 可能在 Provider 解析前就被第一方平台条件拒绝 |
| Materialized Registry 只有 Package、Capability、Skill、MCP Tool，没有角色实现锁 | `crates/backend/nomifun-agent-kernel/src/materialize.rs` | Snapshot 无法表达“同一 canonical Browser 能力本次由哪个实现执行” |
| Resolved Snapshot 冻结 Capability、Skill、MCP Tool 和资源，但不冻结 Browser/Computer Provider | `crates/backend/nomifun-agent-contracts/src/preset.rs` | 全局 Provider 改变后，旧 Session 的执行实现无法被准确恢复 |

#### 2.3 如果全部留到二期会发生什么

如果一期先把 Browser/Computer 直接接到当前具体实现，二期再做替换，将至少重新修改：

- Package contribution schema 和 target inventory；
- Materializer、Registry 和 generation digest；
- Preset Compiler 与 Resolved Snapshot；
- Browser/Computer canonical execution routes；
- Gateway、Agent、Cron、AutoWork、Requirement 等消费者；
- Session 恢复、D-025 compatibility 和 provenance；
- Browser lane 清理与 Computer 全局串行测试；
- C8/C10 的 Browser/Computer 平台验证。

这不是普通的二期增量，而是对一期核心调用链的第二次重构。当前正式 C8-WIN-PRE 证据尚未闭合，且 Browser/Computer 真实 owner 仍未接入，因此应在一期候选冻结前完成本接缝。

### 3. 概念模型

#### 3.1 Execution Role 不是 Agent 人设角色

本文的 Role 只表示“系统需要哪类执行能力”，与 Agent Persona、Preset 角色或用户身份无关。建议公开中文统一称“系统能力角色”，机器合同使用：

```text
system.browser_use
system.computer_use
```

如果 `Role` 容易与 Agent Role 混淆，产品 API 可以使用“Browser 实现”“Computer 实现”，但机器层仍保留统一 `ExecutionRoleId`。

#### 3.2 五个必要概念

| 概念 | 含义 | 是否进入 Agent 能力目录 |
|---|---|---|
| `ExecutionRole` | 系统需要的稳定能力角色 | 否 |
| Canonical Capability façade | Agent 看见并调用的稳定 `browser.*` / `computer.*` 能力 | 是 |
| `RoleProviderContribution` | 某个 Package 对一个 Role Contract 的具体实现 | 否，是 Capability 内部实现贡献 |
| `RoleProviderSelection` | 在创建 Snapshot 前为 Role 选择一个精确 Provider | 否 |
| `ResolvedRoleProviderLock` | Snapshot 中冻结的精确实现、合同和来源事实 | 否，但参与 Snapshot digest |

这不是第五种顶层产品对象。用户仍然只安装 Package、选择 Capability/Skill、配置 MCP；Provider 是 Browser/Computer façade 的内部实现贡献。

#### 3.3 来源、执行方式和 Skill 必须分开

“Plugin、MCP、CLI、Skill 都能提供 Browser/Computer”是产品语言；机器模型不能把四者塞进一个 `ProviderSource` 枚举：

- provenance 回答“由哪个 Package/Mount/Artifact 发布”；
- execution binding 回答“Host 最终怎样调用它”；
- Skill 回答“模型遵循什么说明和工作流”。

纯 Skill 没有执行器，不能直接点击页面、移动鼠标或注册 Provider。合法组合是同一个 Package 同时贡献：

```text
RoleProviderContribution   # 可执行部分
SkillContribution          # instructions/workflow/resources
```

或者 Skill 显式依赖已经选择的 Browser/Computer Capability。选中 Skill 不得自动安装、绑定或授权任何 Provider。

#### 3.4 最小调用图

```text
Agent / Cron / AutoWork / Requirement / MiniApp / Remote
                         │
                         v
          canonical browser.* / computer.* façade
                         │
                         v
                 RoleDispatcher
                         │
              Snapshot exact Provider lock
                         │
                         v
       Capability Registry 内部 role_provider_index
                         │
       ┌─────────────────┴─────────────────┐
       v                                   v
bundled first-party Provider       future Plugin/MCP/CLI Provider
```

Runtime 只执行 Snapshot 已解析的 canonical Capability，不在 turn 中搜索、评分或猜测 Provider。

#### 3.5 “替换实现”与“切入流程”是两种组合方式

用户 DIY NomiFun 不只包含替换 Browser/Computer，也包含让 Plugin 在系统流程中追加行为。两者不能混成一种万能 Hook：

| 组合方式 | 适用问题 | 现有/新增机制 |
|---|---|---|
| 单实现替换 | “当前 Browser/Computer 由谁实现” | 本文新增的 exact Role Provider binding |
| 多贡献组合 | “模型请求前增加上下文”“消费已提交事件”“在 turn 中增加业务行为” | 既有 `context_contributor`、`turn_middleware`、`event_source/event_consumer` 等 Capability kind |

一期继续保证既有 contribution kind 走公共 Package、Snapshot 和 Runtime 主链；二期可以通过 JS Plugin SDK 把它们开放给用户。本文不新增 `before_anything/after_anything` 一类任意字符串 Hook，也不为 middleware 建优先级规则、条件 DSL 或通用编排图。

如果后续需要一个现有 contribution kind 无法表达的新切入点，应先以真实产品场景定义一个稳定、命名明确的 typed extension point，再加入 Capability contract；不能让 Plugin 直接订阅 Kernel 内部函数或任意数据库变化。

UI Shell/Workbench 布局替换属于 UI contribution 与 Surface 架构；Browser Engine/Embedded Surface 属于另一个 ADR。它们都不应借 Browser/Computer Role Provider 顺带实现。

### 4. 架构方案与推荐

#### 4.1 方案一：每种实现发布自己的一套 Browser/Computer Capability

例如同时存在：

```text
browser.observe
acme.browser.observe
playwright_mcp.browser.observe
```

产品影响：用户需要在每个 Agent 中替换整套 Tool，系统内建消费者仍可能固定调用 `browser.*`。

技术影响：实现最少，但 Provider 来源泄漏进 Capability 语义；Skill、Preset、Cron 和 MiniApp 无法稳定依赖“当前 Browser”；不能满足真正的系统级替换。

#### 4.2 方案二：唯一 canonical façade + 精确 Provider binding（推荐）

`browser.*` / `computer.*` 继续是唯一 canonical Capability。Provider 不注册或抢占这些 ID，只在自己的 Package Mount 下贡献一个 Role 实现；exact identity 由 Role、Package、Mount 和 contribution digest 构成。

产品影响：用户替换的是“Browser Use 当前实现”，所有消费者同步受益；Agent 和 Skill 不需要因 Provider 变化修改 Tool 名称。

技术影响：一期需要增加很窄的 Role Contract、现有 Registry 内部索引、Resolver、Snapshot lock 和 dispatch route，但不需要通用图、优先级或自动 fallback。二期只扩展 Provider 接入和选择策略。

#### 4.3 明确排除

以下做法不作为候选：

- 多个 Package 抢占同一个 Capability ID；
- 按安装顺序、来源类别或健康分自动选赢家；
- Provider 失败后静默回退第一方实现；
- Runtime 根据 Tool 名称、描述或 AI 猜测动态映射；
- 在每个消费者旁边增加 `built-in | plugin | mcp` 分支；
- 建立通用 Provider DAG、SAT solver、chain、middleware 或任意生命周期 Hook；
- 为未来 CLI/Skill 在一期增加不可执行的枚举和占位 UI。

已确认采用方案二。

### 5. 一期必须冻结的机器合同

#### 5.1 Role 与 Contract identity

建议新增以下 source-neutral primitive：

```rust
struct ExecutionRoleId(String);       // 首批登记 system.browser_use / system.computer_use

struct RoleContractKey {
    role_id: ExecutionRoleId,
    contract_version: VersionString,
}

struct ExactRoleContractRef {
    key: RoleContractKey,
    contract_digest: DigestHex,
}

struct ExactRoleProviderRef {
    role: ExactRoleContractRef,
    package: PackageRef,
    mount_id: PluginMountId,
    contribution_digest: DigestHex,
}
```

约束：

1. `ExecutionRoleId` 是稳定 namespaced ID；首批 target manifest 只登记 Browser、Computer；
2. 新增其他 Role 需要显式定义版本化合同，不允许 Plugin 自创字符串并挂入任意系统位置；
3. 一个 Package Mount 对同一个 Role 首版最多贡献一个 Provider；若未来出现“一包同时提供同角色多个实现”的真实需求，再增加 local key，不在一期预埋；
4. Provider source kind 只作为 provenance，不参加选择优先级；
5. 同一 Registry generation 中 `(role_id, mount_id)` 重复必须使新 generation 整体失败，旧 generation 保持有效；
6. Manifest payload 不包含自己的 digest；digest 位于 artifact envelope 和 materialization 后的 exact ref，避免自引用。

#### 5.2 Role Contract 不是任意 Tool bag

Browser/Computer 都有资源和生命周期语义，不能只检查几个相似 Tool 名称。

建议 `RoleContractManifest` 至少冻结：

```rust
struct RoleContractManifest {
    key: RoleContractKey,
    members: Vec<RoleMemberContract>,
    serialized_target_resource_kind: Option<ResourceKind>,
}

struct RoleMemberContract {
    capability: CapabilityRef,
    capability_manifest_digest: DigestHex,
    requirement: RequiredOrOptional,
}
```

Role Contract 只登记 canonical Capability member、对应完整 `CapabilityManifest` digest、required/optional，以及 Computer 真正需要的 target-resource serialization；Tool Schema、EffectClass、错误与取消合同继续以现有 `CapabilityManifest` 和统一调用协议为唯一事实源，不在 Role Contract 复制第二份。Materializer 必须校验每个 member 的 exact Capability/version/manifest digest。

不定义通用 lifecycle/concurrency policy enum。Browser lane/owner/close 继续是 Browser v1 的具名 conformance；Computer v1 只使用一个实际会被 Kernel 消费的 `serialized_target_resource_kind`，按 Snapshot 中的 exact target `ResourceId` 串行调用。

Provider 可以声明只支持可选 member 的子集，但不能改变 canonical Capability 的 EffectClass、Schema 或调用语义。

Provider 自有扩展能力必须使用自己 namespace 的普通 Capability，不能偷偷扩张系统 Role Contract。

#### 5.3 Browser Use v1

一期以现有 canonical ID 为 façade，不改成某个 Provider 专属 ID：

| 类型 | Canonical Capability |
|---|---|
| 基线 | `browser.observe`、`browser.navigate`、`browser.act` |
| 可选系统集成 member | `browser.identity`、新增 `browser.render_content` |
| 可选特性 | `browser.download`、`browser.upload`、`browser.evaluate`、`browser.site_memory`、`browser.takeover` |

一期 first-party Provider 必须保持现有完整能力并补齐 `browser.render_content` façade；未来第三方 Provider 至少满足基线才可被识别为 Browser Provider。`browser.identity` 不能作为强制基线阻止没有 NomiFun lane identity 的 MCP/CLI Provider；Provider 缺少系统集成或可选 member 时，由 Compiler/影响分析精确显示受影响 Capability，不把整个 Provider 伪装成不可用。

当前 Knowledge 网页导入的 `BrowserFetcher -> BrowserSessionHub -> navigate/rendered_html` 是已知第一方旁路。它必须改为依赖 `browser.render_content` 并经过同一个 resolved Provider；否则用户替换 Chat 中的 Browser 后，Knowledge 仍会暗中启动第一方 Chromium，不能宣称全局替换完成。

`browser.render_content` 是给 Knowledge 等系统消费者使用的 canonical hidden Tool member：`ToolPresentationKind::Hidden`，不进入默认模型 Tool 面。其最小输入是一个 URL，最小输出是 `final_url + html`，EffectClass 与 canonical navigation 的外部传输语义一致；Provider 在内部完成 navigate/render transaction，消费者不能依赖第一方 lane 或 `rendered_html` 私有 action 名。

Browser lifecycle 保留现有 session/resource-scoped lane、owner lease、close/cancel 和进程回收语义。Role 抽象不能把它降级为无状态 Tool bag。

#### 5.4 Computer Use v1

一期以现有 canonical ID 为 façade：

| 类型 | Canonical Capability |
|---|---|
| 基线 | `computer.observe`、`computer.input` |
| 可选特性 | `computer.launch`、`a11y.observe` |

`a11y.observe` 不是因为当前与 Computer 共包而被隐式吞并，而是作为 Computer Use 的可选观察方式显式进入 v1 合同；不支持 A11y 的 Provider 仍可满足基线。未来如果 A11y 出现脱离 Computer 的独立系统消费者，再另立 `system.accessibility` Role，不在一期预建第三个 Role。

Computer Role 只保证同一 target instance 上的每次物理 action 串行，不承诺两次 Tool 调用之间的多调用原子事务。观察结果必须携带 observation generation；Provider 在使用 element ref 时校验 generation，过期就 typed fail 并要求重新 observe。第一方 Provider 只有一个本机物理桌面；未来远程或多设备 Provider 可以提供多个 target instance，每个 instance 分别遵守该规则。

#### 5.5 RoleProviderContribution

一期 canonical Rust contract 建议包含：

```rust
struct RoleProviderContribution {
    role: ExactRoleContractRef,
    display: LocalizedMetadata,
    members: BTreeMap<CapabilityId, RoleProviderMemberContribution>,
}

struct RoleProviderMemberContribution {
    supported_platforms: Vec<PlatformConstraint>,
    required_resource_kinds: BTreeSet<ResourceKind>,
}
```

资源和平台约束按 member 声明，Compiler 只合并当前 Agent ceiling 或 non-Agent operation 实际使用的 member；不能因为 Provider 支持 `takeover` 或 `launch`，就强迫只使用 observe/navigate 的调用方绑定可选资源。

同一可序列化 `role_providers[]` shape 必须在一期进入 `PackageContributions`、target first-party inventory 和 generated schema。Phase 1 的 bundled Rust registration 使用它；二期 Node Plugin loader 与 MCP Adapter 只能 materialize 同一 shape，不能新增第二套 JS/MCP Provider schema。

执行对象不以 `Plugin | MCP | CLI | Skill` 枚举进入 Kernel。`PluginRegistration` 在内存中同时登记 metadata 与按现有 `CapabilityKind` 区分的窄 typed exports：

```rust
struct RoleProviderExports {
    action_members: BTreeMap<CapabilityId, Arc<dyn CapabilityHandler>>,
    context_members: BTreeMap<CapabilityId, Arc<dyn ContextContributionFactory>>,
    resource_members: BTreeMap<CapabilityId, Arc<dyn ResourceProviderFactory>>,
}
```

Browser/Computer v1 只要求 Tool、ContextContributor、ResourceProvider 三类执行接缝；不为了未来角色预建 Event、Transport、Scheduler 或任意 Hook executor。未来 Node Host、MCP Adapter 或 managed CLI Host 都在 Kernel 外把自己的执行方式适配到对应 typed export。

`ContextContributionFactory` 与 `ResourceProviderFactory` 是一期需要补齐的新 canonical runtime seam，不是对当前实现能力的描述：当前 Registry 只要求带 action 的 Capability 提供 handler，Context/Resource 主要仍是 manifest metadata。P1-R0/R1 必须先冻结并实现这两个最小接口，不能用现有 metadata-only 行为宣称 Role Provider 已覆盖三类 member。

只提供 action handler 不算完成 Browser/Computer 替换：`browser.identity` 的资源解析、`browser.observe` / `computer.observe` 的 Context 组装也必须读取同一个 frozen Provider lock。否则只会替换可见 Tool，Context 和 Resource 仍走第一方旁路。

第一方与未来第三方的差异只能来自 Package、Mount、Artifact、配置和执行 adapter；Materializer、Resolver、Snapshot 和 Dispatcher 不得根据 `Bundled` 特判。

#### 5.6 Registry、Binding 与选择策略

一期新增的 Registry 只是平表，不是 Provider graph：

```text
(ExecutionRoleId, PluginMountId) -> MaterializedRoleProvider
```

已确认一期采用“安装级默认 + Agent 显式覆盖”，并在 Fresh-v4 正式封板前把最小 Binding 机器合同一次加入 clean schema，避免二期再迁移 Agent Revision、Snapshot 和数据库：

```rust
struct RoleProviderSelection {
    role: ExactRoleContractRef,
    provider_mount_id: PluginMountId,
}

struct InstallationRoleBinding {
    selection: RoleProviderSelection,
    binding_version: u64,
    updated_at_ms: i64,
}

AgentPresetRevisionPayload {
    // existing fields...
    system_role_provider_overrides:
        BTreeMap<ExecutionRoleId, RoleProviderSelection>,
}
```

Agent override 不存在就表示继承 installation default；不增加 `inherit/default/latest/follow` 状态枚举。Binding 指向稳定 Mount 和 exact Role Contract，不锁死 Package version；Compiler 在创建 Snapshot 时把当前 Mount materialize 成 exact Package/version/contribution digest。

Fresh-v4 只增加一张 installation-wide 表，每个 Role 一行，保存 role contract ref、provider mount、binding version 和更新时间。它不构成 Provider Catalog，也不加候选、优先级或历史表。Phase 1 seed 固定：

```text
system.browser_use  -> bundled first-party Browser Mount
system.computer_use -> bundled first-party Computer Mount
```

Resolver 顺序固定为：

```text
Agent Revision exact override
→ installation default binding
→ missing typed failure
```

一期不提供用户修改 installation binding 的 API/UI，也不展示 Provider picker；但 Temp DB、Compiler 和 Snapshot 测试必须能够把默认或 Revision override 指向 alternate fixture，证明这不是写死第一方 ID 的假 Binding。二期只增加 mutation application service 和产品 UI，不再改变表、Revision 或 Snapshot 语义。

Binding 表不使用会阻止 Package Disable/Uninstall 的硬外键。目标暂时不存在时保留用户选择，由 Resolver 返回明确不可用；不能因为 Provider 缺失自动改写成第一方 Mount。

选择规则：

- 一次 resolution 对每个需要的 Role 只能得到一个 exact Provider；
- 零个结果返回 `ROLE_PROVIDER_NOT_BOUND`；
- exact Provider 不存在或合同不匹配返回 `ROLE_PROVIDER_UNAVAILABLE`；
- Registry 中存在多个候选不会触发自动选择，只有 override/default 指向的 Mount 生效；
- Provider 暂时失败是普通调用失败，不触发换 Provider；
- 安装顺序和 source kind 不得影响结果。

#### 5.7 Snapshot 必须冻结实际实现

建议在 `ResolvedSnapshotContent` 增加：

```rust
struct ResolvedRoleProviderLock {
    provider: ExactRoleProviderRef,
    source: PluginSourceMetadata,
    supported_members: BTreeSet<CapabilityId>,
    resource_binding_refs: Vec<ResourceBindingId>,
}

resolved_role_providers: BTreeMap<ExecutionRoleId, ResolvedRoleProviderLock>
```

该字段参与 Snapshot canonical digest。Compiler 流程固定为：

```text
resolve canonical Capability ceiling
→ derive required Execution Roles and members
→ select exact Provider for each Role
→ validate member/platform/resource compatibility
→ freeze ResolvedRoleProviderLock
→ create AgentSession
```

这会改变当前“先按 canonical Browser/Computer Capability 上的第一方平台/资源约束判定，再接具体 Host”的顺序。新的唯一顺序必须是“先由 override/default 选 Provider，再按该 Provider 的 member platform/resource 约束编译”；否则 Headless MCP/远程 Provider 仍会在 Provider 选择前被第一方条件拒绝。

运行期不得重新选择 latest Provider。若 exact Provider artifact、contract 或 contribution 已不存在或结构不兼容，沿用 D-025：原 Session 保持只读并返回 `SNAPSHOT_EXECUTOR_UNAVAILABLE`；用户可以显式 fork child 或创建新的 AgentSession，原 Session 不改写，也不得静默换实现。Provider 进程暂时 crash、MCP 临时断线、Credential 或资源暂时不可用属于普通运行错误，不得把 Session 错误标成结构不兼容。

一期冻结 Binding 数据结构、Resolver 和解析结果，但不建设面向用户的 Provider 管理产品。二期开放选择和切换时，Snapshot 和 Runtime 调用合同保持不变。

#### 5.8 Non-Agent 系统操作的 exact binding

`ResolvedRoleProviderLock` 必须是可以独立传递的 canonical value，不能只能藏在 Agent Snapshot 内。两类调用使用同一个 Resolver 和最终 dispatch primitive：

- Agent/automation Session：从 frozen Snapshot 取得 exact lock；
- 没有 AgentSession 的系统业务操作：在 operation admission 时从 installation binding 解析一次 exact lock，连同 principal、owner、operation ID 和 typed resources 放入该 operation context；若业务操作本身可跨 crash 恢复，则把同一 lock 随其 durable operation record 保存。

Kernel 内部最终只保留一个 `dispatch_resolved_role_member(lock, context, member, input)`。Agent `KernelRegistry::invoke` 负责从 Snapshot 取 lock；Knowledge URL import 等 typed application service 负责在 admission 时取 operation lock。两者不得在执行过程中重新读取“当前默认 Provider”。

这样 `browser.render_content` 可以覆盖没有 AgentSession 的 Knowledge 导入，又不会制造另一个 Provider 选择规则。人类 Browser management/login/Surface 是第一方 Browser 产品控制面，不属于这一 Role invocation，不需要伪造 Agent Snapshot。

#### 5.9 Provider 与资源绑定

Canonical Browser/Computer façade 不得硬编码第一方实现特有的资源：

- 第一方 Browser Provider 可以要求 Browser lane/profile binding；
- 第一方 Computer Provider 可以要求 physical desktop binding；
- 未来 MCP Provider 可以要求 MCP server/connection binding；
- 未来 Plugin Provider 可以只使用自身配置和 Credential，或声明其他 typed resource。

Provider 在 materialization 时按 member 声明 resource kinds，Compiler 只为实际选择的 member 解析 exact binding refs，Dispatcher 只得到 Snapshot 或 non-Agent operation 已冻结的资源。不得把 Provider identity 或任意 JSON 配置塞进 `TypedResourceBinding.typed_parameters` 形成 stringly-typed 旁路。

Compiler/activation 只校验和冻结 resource binding refs，不得在 resolve、Session create 或 on-demand turn-boundary 仅因“可能会用”就启动 Browser、MCP 或其他 Provider。`ResourceProviderFactory` 只在首次真实 member 调用或 Context 组装确实需要时 lazy acquire，并登记现有 `ResourceHandle`，随 Session/non-Agent operation teardown 清理；Context factory 同样使用 frozen Provider Mount 的 context，不回到 façade Mount。

平台可用性属于具体 Provider，而不是 Role 的永久全局禁令。D-028 的 Headless Browser/Computer unavailable 继续约束一期 first-party 默认和首个 Stable；未来 Provider 若明确支持 Headless，可以在后续阶段通过自己的 `supported_platforms` 和原生 Gate 扩大产品能力，而不修改 Role Contract。

Role-backed canonical Capability 只能声明该能力本身允许出现的 surface/target 语义；实际 availability 必须在选择 Provider 后按“Role semantic constraint ∩ Provider supported platforms”计算。不得继续用第一方 `computer-use` build feature 或 native Browser 是否编译来提前判定整个 Role 不可用。

#### 5.10 RoleDispatcher

`RoleDispatcher` 是现有 Kernel Registry/Compiler 内部的一条 exact route，不是独立 Service、Package 或第二次 Capability 调用。

Tool 调用固定为：

1. `KernelRegistry::invoke` 继续先对 canonical Capability 执行现有 ThinAuthority；
2. 在选择 handler 时，根据 canonical Capability 找到 Role，并从传入的完整 `CompiledSnapshot` 读取 `ResolvedRoleProviderLock`；
3. 在同一个 published Registry generation 的 `role_provider_index` 中找到 exact Provider member export；
4. 使用 **Provider Mount** 的 config、state、service view 和 Snapshot resource bindings 构造 invocation context；
5. 保持原 canonical Capability ID、Action ID、Operation/Effect/Idempotency identity，只调用一次 Provider handler；
6. 原样返回 canonical result 或 typed error，不查找第二个 Provider。

不能先调用一个 façade Package handler，再由它回调 Kernel 查 Provider：当前 handler context 只有 Snapshot ref，而且已经绑定 façade Mount 的 state/service view，这会形成 Kernel ↔ Plugin 双分发和错误的 owner context。Role-backed façade 在 Registry 中是声明式 Capability contract，真正的 handler 在 Kernel 第一次选择时直接解析到 frozen Provider Mount。

ContextContributor 在 Context Assembler 选择 factory 时读取同一个 lock；ResourceProvider 在资源解析/实例化时读取同一个 lock。三类路径共用 exact Provider 解析规则，但不伪装成同一种 action handler。

Computer physical action 在进入 Provider handler 前，按 `serialized_target_resource_kind` 对 Snapshot 的 exact target `ResourceId` 取得一次调用级共享 arbiter；锁属于 target，而不是某个 Provider。它只串行单次 action，不跨 observe→think→input 的模型间隔持有长 lease；ref 是否过期由 observation generation 校验。Raw trusted code 绕过正式 Provider API 不在本合同保证范围。

该 route 不做发现、安装、评分、fallback、重试、链式 Provider 或 AI 决策。

#### 5.11 Provenance 与事件

每次真实调用至少能关联：

- canonical Capability ID；
- Execution Role ID 和 contract digest；
- exact Role contract、Package version、Mount 和 contribution digest；
- source provenance；
- Snapshot ID、Session ID、operation/effect identity。

不要求一期新增面向用户的 Provider 历史页面。现有 SessionEvent/EffectReceipt 若已经能够引用 Snapshot 和 Capability，只需保证通过 Snapshot 可确定性还原 Provider；不要复制第二份可漂移的详情。

### 6. 一期干净实现与删除边界

#### 6.1 Contract 与 Materializer

建议影响范围：

- `nomifun-agent-contracts`：新增 Role/Provider/Lock canonical types 和 schema；
- `nomifun-agent-kernel`：Materializer、现有 Registry 内部 `role_provider_index`、selection port、Compiler lock 和 exact dispatch route；
- contract generator 与 checked-in schema：随 canonical source 一次生成；
- target first-party inventory：登记 Browser/Computer Role Contract、默认 Provider 与 digest。

不得：

- 在 API DTO、DB 或 UI 先建立二期 Provider 管理对象；
- 用 `StrictJsonValue`、字符串 map 或 `typed_parameters` 假装完成 typed contract；
- 只在 Browser/Computer 两个 handler 内各写一份临时选择逻辑。

#### 6.2 Bundled Browser/Computer

`nomifun-agent-domain-wave2` 继续拥有 canonical `browser.*` / `computer.*` Capability façade，并登记：

- `system.browser_use` / `browser-use-v1`；
- `system.computer_use` / `computer-use-v1`；
- 第一方 exact Provider contribution；
- façade Capability member 到统一 exact dispatch route 的映射。

第一方具体实现仍可复用现有 Browser Hub、lane、ComputerTool 和 Registry，但这些对象只能位于 first-party Provider 后面。

现有 Browser/Computer Capability 上的第一方平台限制应迁入 first-party Provider contribution；`current_host_surface()` 等 Host surface 判断必须表达真实宿主 surface，不能再用某个 first-party feature 是否编译来代替 Role availability。

#### 6.3 Composition Root 与消费者

全局 clean cut 要求：

1. Composition Root 构造第一方 Provider 后，通过普通 Plugin registration/materialization 发布；
2. 新 v4/Codex first-party Provider 不得复用 `BrowserLaneClientProviderSlot`；现有 Nomi-only Slot 不增加 Role Adapter 或兼容层，按 D-020 作为有期限 legacy allowlist 留到 C9 并随 Nomi 整体删除；
3. `ComputerRegistry` 不再出现在 Gateway 或业务消费者的依赖 view 中，只属于 first-party Computer Provider；
4. Wave 2 Browser/Computer 不再走永久 unavailable 分支，而是调用 canonical Dispatcher；
5. Gateway Browser/Computer 入口若仍需存在，只能委托 canonical Agent Platform/Capability 主链，不得直接执行具体 Registry；
6. `mcp-computer-stdio` 不得继续自行构造 `ComputerTool`；Codex/ACP 要么通过带 AgentSession/Snapshot 的 canonical Host route 使用 Computer，要么删除该 standalone direct bridge；
7. Chat、Agent、Cron、AutoWork、Requirement、Remote、Knowledge、MiniApp 及其他 Agent/automation 消费者不得新增 Browser/Computer concrete crate/service 依赖；
8. Knowledge `BrowserFetcher` 的 rendered HTML 路径必须改用 `browser.render_content`，且不能在 Provider 缺失时回退第一方 Hub。

#### 6.4 必须删除的旧形态

实现完成时应删除，而不是保留 fallback：

- consumer-facing built-in Browser/Computer service fields；
- Gateway 对 `ComputerRegistry.execute` 的直接生产调用；
- `mcp-computer-stdio` 对 `ComputerTool::new` 的直接构造与执行；
- 新 v4/Codex Agent Factory 对 native Browser slot 的任何引用；legacy Nomi-only Slot 到 C9 直接删除，不先改造；
- `if plugin/mcp then ... else builtin ...` 分支；
- 同一操作的 legacy Gateway Registry 与 canonical Capability 双执行入口；
- Provider 缺失时回退旧 Browser/Computer 的代码；
- 仅为过渡存在的 alias、shadow ID、feature switch 和兼容 adapter。

底层 Browser Hub、ComputerTool 和 lane cleanup 不是历史债；它们作为第一方 Provider 内部实现继续保留。Computer 的跨 Provider target arbiter 位于公共 exact route，第一方 Provider 不再独占全局排序事实。

人类 Browser 管理、登录、诊断、Surface、进程生命周期、telemetry 和 shutdown 是 Browser Engine/产品控制面，不是 Agent/automation Browser Use 消费者；这些具名 owning surface 可以直接持有 `BrowserSessionHub`。允许清单必须精确到模块/用途，不能把 Knowledge、Agent Factory、Gateway 或自动化消费者归入“管理”例外。

P1-R2/C8 的 residual-zero 只针对新的 v4/Codex、Knowledge、Gateway 和 stdio target 主链。D-020 deletion manifest 中已经登记的 Nomi-only Browser/Computer wiring 可以作为精确、有期限的 legacy allowlist 保留到 C9，但不得增长或接入新架构；C9 删除 Nomi 后，除上述人类 Browser owning surface 和 first-party Provider 实现外，全仓具体实现旁路才要求为 0。

### 7. 一期与二期的精确边界

#### 7.1 一期 05 必须完成

- Browser/Computer 两个 versioned Role Contract；
- canonical façade 与 Provider implementation identity 分离；
- source-neutral `RoleProviderContribution` canonical Rust contract，并把同一可序列化 shape 加入 `PackageContributions`、target inventory 和 generated schema；二期只能由 Node loader 复用，不能另造 JS 专用 shape；
- 现有 Materialized Registry 内部的 flat `role_provider_index`；
- Fresh-v4 installation binding 表、Agent Revision override 字段、first-party seed 和固定解析顺序；
- `ResolvedRoleProviderLock` 进入 Snapshot digest；
- Kernel action route 与一期新补齐的 Context/Resource runtime stages 共用同一 exact Provider lock；
- 第一方 Provider dogfood 同一 materializer/registry/dispatcher；
- Browser/Computer 所有生产消费者无具体实现旁路；
- 一个 test-only alternate Provider 证明可替换；
- C8/C10 受影响 Gate 和 contract digest 更新。

#### 7.2 一期明确不做

- Node Runtime Manager、Extension Host、JavaScript SDK；
- 用户 Plugin 安装、启停、Replace、Uninstall；
- 面向第三方公开 Role Provider Manifest/SDK；
- MCP Role Adapter；
- CLI Host 或 CLI Provider protocol；
- Skill 与可执行 Provider 的组合产品体验；
- 面向用户修改 installation binding 的 mutation API；
- Agent Editor Provider override UI；
- “系统能力实现”设置页、影响清单和切换流程；
- Chat Dev Provider 模板和 conformance runner；
- Provider 市场、评分、自动择优、健康调度或 fallback；
- IDMM、模型路由、UI Shell 等其他 Role 的本轮迁移；
- Browser Engine 与 Embedded Surface 选型。

#### 7.3 二期 06 的前置条件与后续边界

05 的 Role/Provider 合同落地后，仍必须先完成本文 §15 的 AgentPreset 平台级能力建设；只有 AP-0～AP-7 全部通过，06 才能进入机器合同冻结和代码实施。06 的产品决策可以在本轮同步修订，但不能绕过该前置条件：

1. 在 Node Plugin SDK/loader 中公开并复用一期已冻结的 Browser/Computer Provider contribution shape；
2. Extension Host 把 JS Provider 适配到一期统一的 typed Provider exports；
3. 标准 MCP Toolset 直接映射；Schema 不一致时由 Chat Dev 生成薄 JS Adapter Plugin；
4. 为一期已冻结的 installation binding 和 Agent Revision override 增加统一 application service；如需暴露精确实现选择，只能作为 Agent 工作台能力详情中的高级动作，不得再建一个“运行时 Agent 设定”产品；
5. 在同一个 Agent 工作台中提供来源/provenance、Test、影响清单、切换和 Restore Built-in；Runtime Manager 仍是系统基础设施设置，不属于 AgentPreset 内容；
6. 新 Session 使用新 Provider，已有 Session 保持 frozen Snapshot；
7. Provider 失败时提供 Retry、Switch、Restore，不静默 fallback；
8. CLI 首期通过 Plugin Adapter 接入，待 managed stdio protocol 成熟后再成为一等 binding；
9. Skill 始终是 instruction/workflow；需要可执行部分时与 Plugin/MCP/CLI Provider 同包或显式组合；
10. Chat Dev 支持“创建 Browser Provider / 创建 Computer Provider / 为 MCP 生成 Adapter”的一站式流程。

其中“标准 MCP Toolset 直接映射”仍必须先经过既有 `McpToolCapabilityMapping`、exact schema digest、resource binding 和 provenance lock；Role Adapter 只是把已物化的 backing MCP capabilities 适配为 canonical Role members，不能让裸 MCP Tool 形成第二身份或绕过 Snapshot 的 MCP lock。

06 当前“不能透明 override 内建能力”的表述以后应改成：

> 禁止同 Capability ID 抢占和静默劫持；允许用户通过显式 Role Binding 替换系统能力的当前实现。

### 8. 实施顺序

本文发布后，一期实现使用一个完整切片，不做临时中间产品：

#### P1-R0：合同冻结

- 冻结 `ExecutionRoleId`、Role Contract、Package/Mount Provider identity、Contribution、Snapshot lock 和 typed errors；
- 更新 canonical schema generator、target inventory 和 decision contract digest；
- 为 Browser/Computer 现有 capability 集建立 required/optional member 映射，并新增 `browser.render_content`；
- 冻结 Context/Resource 两类最小 runtime export，以及 installation binding/Agent Revision override shape；
- 明确 `a11y.observe` 只是 Computer v1 的 optional member，不因共包变成基线或独立第三 Role。

退出条件：canonical sources、generated artifacts 和文档不存在两套字段定义。

#### P1-R1：Registry、Resolver 与 Snapshot

- Materializer 接收 Provider contribution；
- Registry generation 发布内部 flat `role_provider_index`；
- Fresh-v4 seed installation bindings，Revision absence 表示继承默认；
- AgentPreset Compiler 先解析 override/default Provider，再计算实际 member platform/resource 并写入 Snapshot lock；
- non-Agent operation admission 生成同一种 exact lock；
- D-025 compatibility 比较 Provider/contract/contribution digest。

退出条件：test fixture 能选择 alternate Provider 并生成不同但自洽的 Snapshot digest。

#### P1-R2：Browser/Computer clean cut

- bundled Browser/Computer 注册 façade 和 first-party Provider；
- canonical Tool、Context 和 Resource member 全部按 Snapshot lock 选择 Provider export；
- 具体 Hub/Registry 收回 Provider 内部；
- Knowledge `render_content` 使用 non-Agent operation exact lock；
- 删除 Gateway、Factory、Service bag、`mcp-computer-stdio` direct tool 和业务消费者旁路；
- 保留 Browser lane cleanup，并把 Computer ordering 收敛为按 exact target resource 的共享 arbiter。

退出条件：v4/Codex target 生产依赖扫描中，只有 first-party Provider implementation，以及具名的人类 Browser 管理/登录/Surface/lifecycle owning modules 可以引用具体 Browser backend；Computer backend 只允许 first-party Provider 引用。v4/Codex Agent/automation、Knowledge、Gateway 和 stdio bridge 的具体实现旁路为 0；Nomi-only exact allowlist 不增长并明确等待 C9 删除。

#### P1-R3：一期 Gate 收口

- targeted contract/materializer/compiler/dispatcher tests；
- Browser/Computer first-party 与 alternate fixture parity；
- Snapshot resume/unavailable/no-fallback；
- Browser owner/lane cleanup、Computer 串行与平台 unavailable；
- 受影响的 Windows C8 Gate；
- 后续 macOS/Linux 原生 Gate 按既有 D-028 批次执行，不因本文建立新的逐功能换机流程。

退出条件：本次 Browser/Computer 变更已经进入当前 candidate source；实际 Host、Sidecar 和 Package digest 记录于 release lock，对应首发三平台结果记录于 platform result；旧证据不得冒充本次结果。

### 9. 最小验证矩阵

| 层级 | 必须证明 |
|---|---|
| Contract | Role、Package/Mount Provider identity、digest、required/optional member、typed error exact-set |
| Materializer | first-party 与 fixture 使用同一入口；duplicate Provider hard-fail；Capability ID 无抢占 |
| Resolver | Revision override 优先、absence 继承 installation default；缺失/不兼容明确失败；source kind 和安装顺序不影响选择 |
| Snapshot/Operation | exact Provider/contract/resource lock 进入 digest 或 operation context；执行中不重新 resolve latest |
| Dispatch | Tool、Context、Resource 都只读取同一 frozen Provider；使用 Provider Mount context；不产生 façade→Kernel 二次调用；无 fallback/retry |
| Browser | owner/lane/profile/cancel/close 保持；`browser.*` 和 Knowledge `render_content` 全部经 exact route |
| Computer | first-party 单桌面行为保持；两个 Provider 绑定同一 target fixture 时由 target arbiter 串行 |
| Consumer | Chat/Agent/自动化/Remote/Knowledge 等生产入口不引用具体实现；替换 fixture 不改消费者代码 |
| Platform | D-028 first-party 默认保持 Headless typed unavailable；Provider availability 不再被 first-party build feature 提前锁死 |
| Residual | v4/Codex built-in shortcut、dual registry、same-ID override、source switch、legacy fallback 为 0；Nomi-only exact allowlist 不增长并在 C9 删除 |

不要求一期测试 Node Plugin、MCP Schema drift、CLI 进程协议或 Skill 组合；这些属于二期。

### 10. 不允许的“补丁式实现”判定

出现以下任一情况，应停止当前切片并重新做结构设计：

1. 只在 Browser/Computer 当前 handler 外再包一层名字叫 Dispatcher 的转发器，但消费者仍可直接调用具体服务；
2. 第一方路径不经过 `role_provider_index`，只有第三方才经过；
3. Snapshot 只记录 `browser.*`，运行时再读取当前全局设置决定 Provider；
4. 用 Capability ID 冲突、注册顺序或 source priority 表达 override；
5. 在 Gateway、Cron、AutoWork、MiniApp 中分别实现 Provider 选择；
6. Provider identity、合同或资源绑定塞入无 schema JSON/字符串参数；
7. 保留旧 Browser/Computer 入口作为“临时 fallback”；
8. 为通过旧测试而增加 legacy alias、双写、双 Registry 或隐藏 feature flag；
9. 把 Skill 当成可执行 Provider，或让 Skill 自动扩张 Snapshot；
10. 因为未来可能支持更多 Role，提前建设通用 Hook/Graph/Policy Engine。

### 11. 对一期进度和远程开发的影响

当前一期处于 C8-WIN-PRE，正式全量证据尚未闭合，Browser/Computer 真实 owner 仍为 typed unavailable。本变更会修改 canonical contract、Snapshot、Materializer 和 Browser/Computer 接线，因此不能作为 C8 之后的小补丁混入：

- 应先完成本文验收；
- 再把 05 作为独立 Phase 1 amendment 提交；
- 远程 Windows 实施任务在冻结最终 candidate 前完整接收该提交；
- 一次完成 P1-R0～P1-R3，并重新生成受影响合同和 Gate tuple；
- 完成 clean Windows checkpoint 后，再按原计划进入 macOS arm64；
- 不把未提交的二期 06 带入一期实施分支或提交。

如果远程任务已经开始直接接线 Browser/Computer，应暂停该局部接线并按本文重新划分依赖；不在现有直连上继续叠加 Provider Adapter。进入二期 Plugin/MiniApp 实现前，还必须完成文末 §15 的 AP-0～AP-7。

### 12. 完成定义

#### 12.1 设计完成

- 用户已经确认本文方案和一期/二期边界；
- 05 单独进入版本控制；
- 不把未通过 §15 门禁的 06 代码视为一期交付；06 设计可以单独修订，但实现必须等待 AP-0～AP-7；
- 远程实施 Prompt 明确从 05 开始，不从聊天摘要猜测合同。

#### 12.2 一期功能完成

- P1-R0～P1-R3 全部退出；
- first-party Browser/Computer 可通过唯一 façade/Dispatcher 调用；
- alternate fixture 证明 Provider 可换而消费者不变；
- Snapshot 精确冻结实际 Provider；
- production concrete bypass 和 fallback residual 为 0；
- Browser/Computer 原有生命周期与平台语义没有回退。

#### 12.3 不代表二期完成

一期完成不代表用户已经可以：

- 安装 JS Browser/Computer Plugin；
- 把任意 MCP 设为系统实现；
- 在设置或 Agent Editor 中切换 Provider；
- 用 Chat Dev 生成 Adapter；
- 使用一等 CLI Provider，或把 Skill 与可执行 Provider 组合。

这些必须在 06 通过 §15 门禁、正式实现并再次验收后才能宣称。

## 15. AgentPreset 平台级能力建设（Phase N1 前置 TODO）

> 状态：**DESIGN FOLLOW-UP / 06 IMPLEMENTATION BLOCKED**
>
> 本节是用户确认“只保留一个 Agent 工作台”之后，对 AgentPreset 的最终实施合同。它不是在旧“设定”旁边再增加一个新设置页，也不是把 Plugin/MiniApp 改造成 Agent 专属子系统。

### 15.1 产品名称与领域边界

产品层只保留一个一级入口：**Agent 工作台**（导航和按钮可简称为“Agent”）。用户在这里完成 Agent 的能力设计、保存、试用和继续使用；不再保留“旧设定”“Agent 设定”“启动方案”或“运行时 Agent 设定”等并列概念。

`AgentPreset` 只保留为后端的 canonical authoring aggregate。普通用户不需要理解 `Preset` 这个内部名称，也不直接编辑 Revision、Snapshot、Digest、Mount 或 Contribution ID。

本节覆盖 02、`DECISIONS` 及其他前置文档中与公共命名、Package template 或 Agent-only Capability 相关的冲突条款；在 AP-0/AP-6 中完成全量引用清理前，阅读者应以本节为准，不得把旧文案当作新的产品入口。

各领域的 owner 和关系必须固定如下：

| 对象 | 所属领域 | 负责什么 | 与 AgentPreset 的关系 |
|---|---|---|---|
| Package / Plugin | 平台扩展与 Package 域 | 安装、启停、配置、数据、版本和贡献发布 | 贡献能力，但不属于 AgentPreset |
| MiniApp / Active Release | MiniApp 产品域 | Surface、Release、Service、业务数据和发布生命周期 | Active Release 可贡献能力，但不属于 AgentPreset |
| Capability Catalog | 平台能力目录域 | 物化已发布、可用、带 provenance 的平台能力合同 | 被多个消费者查询和解析 |
| Skill Catalog | 平台技能目录域 | instruction、workflow 和资源说明 | Agent 可选择，Skill 不成为执行器 |
| AgentPreset | Agent authoring 域 | 描述一个 Agent 想使用的能力集合和行为 | 是一个消费者声明，不拥有 Capability |
| AgentPresetRevision | Agent authoring 域 | 保存一次不可变的用户设计结果 | 由 Compiler 生成 ContributionLock |
| ResolvedSnapshot | 执行解析域 | 锁定一次实际执行闭包和精确来源 | 被 Session、Binding 或一次性操作消费 |
| AgentSession / AgentBinding | Agent 运行域 | 承载会话、Remote、Automation 等 Agent 使用关系 | 消费 Revision/Snapshot，不反向修改它 |
| Runtime Manager | 系统基础设施域 | Node、Host、进程、Runtime protocol 和恢复 | 不是 AgentPreset 内容，也不是 Agent 的能力选择 |

核心边界是：

1. Capability 的语义 owning domain 是平台 Package、Plugin、MiniApp 或业务域；Capability Catalog 只负责物化索引和可用性视图，不接管能力语义；AgentPreset 不拥有 Capability。
2. 同一个 Capability 可以同时服务 Agent、Gateway/Remote、UI/业务域、Automation、MiniApp/Service 或其他正式平台入口。
3. 某项 Capability 也可以完全不支持 `agent` Surface；Agent 可选性不是平台能力成立的必要条件。
4. Plugin/MiniApp 的安装、替换、发布、Credential、KV、`dataDir` 和 Service 生命周期由各自产品管理；AgentPreset 只保存 typed reference。
5. Agent 工作台可以显示能力来源和可用性，但不接管 Plugin/MiniApp 管理。
6. Runtime 的选择、下载、Host 重建和进程诊断属于系统基础设施设置；它不再以“Agent 设定”的形式出现，也不能被 Agent Revision 自行选择或 fallback。

### 15.2 平台 Capability 与多消费者模型

面向一个或多个消费者的 Plugin/MiniApp Capability、Skill、Tool、Context 或 Resource contribution 必须先进入平台 Capability Catalog，再由不同消费者按各自合同选择。仅作为 canonical façade 内部实现的 `RoleProviderContribution`、Host-only contribution 等，可以进入共享 Materializer/Registry 的内部索引，但不得伪装成用户可选择的独立 Capability。Agent 只是其中一个消费者：

```text
Package / Plugin Release / MiniApp Active Release / Built-in / MCP
                              │
                              v
                  Contribution Materializer
                              │
                              v
                  Platform Capability Catalog
                              │
             ┌────────────────┼────────────────┐
             v                v                v
       AgentPreset       Gateway/Remote     UI/业务/Automation
        Compiler          Operation Lock      Consumer Resolver
             │
             v
      ResolvedSnapshot
             │
       AgentSession / Binding
```

Catalog entry 至少必须能表达以下事实：

- 稳定 `capability_id`、合同版本和 `contract_digest`；
- owning Package/业务域与发布身份；
- `contribution_id`、来源类型、Mount/Release 和 Artifact provenance；
- 支持的消费者或 Surface（例如 `agent`、`gateway`、`remote`、`automation`、`ui`、`miniapp_service`）；
- Tool、Context Contributor、Resource Provider、Skill、MCP-backed Capability 等 typed contribution；
- 所需 Runtime feature、typed resource contract 和当前 availability；
- `active / unavailable / disabled / needs-runtime / contract-mismatch` 等可解释状态。

`supported_surfaces` 或等价字段只表示“该能力可以被哪些平台消费者理解”，不是授权，也不是把 Capability 转移给 Agent。每个消费者仍要经过自己的 policy、resource binding 和 exact resolver。

Catalog 的“已物化”与“对某消费者可用”必须分开：同一贡献可以已经进入平台 Catalog，但只对 UI 或 Gateway 可用、对 Agent 不可用。availability 诊断应按 consumer/surface 给出，不能用一个全局布尔值让 Agent 或其他消费者互相误判。

只有正式发布且当前可执行的贡献才能进入 Catalog：

- Plugin 必须经过 Package current/Enable/Contribution materialization；
- MiniApp 只有 Active Release 的正式贡献可以进入 Catalog；
- Ready Candidate、未发布 Release、未启动 Service、Project Source、Plugin 私有 `dataDir` 和测试 Host 都不能进入 Agent Snapshot 或正式 Catalog；
- Candidate Test 与 Agent Test 是两条不同链：前者验证候选代码，后者创建普通 AgentSession。

### 15.3 最终 AgentPreset 合同

#### 15.3.1 用户可编辑内容

Agent 工作台只允许用户编辑产品语义和 typed 选择：

```text
identity
persona / instructions
model route
selected capabilities
selected skills
selected MCP capabilities
typed resource defaults / overrides
starter prompts
```

其中：

- `selected capabilities` 使用 Catalog 中的能力引用和实际 action allowlist；用户不填写 Package、Mount 或 Artifact；
- `selected MCP capabilities` 指向已物化、已绑定、带 schema/provenance 的 MCP 能力，不接受裸 MCP tool JSON；
- `typed resource defaults / overrides` 只表达工作区、知识库、连接器等正式资源绑定，不保存 Secret 明文或 Plugin 私有数据路径；
- 如果同一 canonical 能力存在多个合法实现，用户只能通过能力详情中的显式“实现来源”高级动作选择，后台由 application service 生成锁；不增加第二个“运行时 Agent 设定”对象；
- 保存和试用都调用同一个 canonical Compiler，前端不自行拼 Snapshot。

#### 15.3.2 系统生成内容

Compiler 必须生成并持久化不可变 Revision 的以下事实：

```text
ContributionLock[]
revision_digest
compiler diagnostics
```

最小持久化关系保持单一主链：

```text
AgentPreset {
    preset_id
    owner_ref
    display
    current_revision_id
}

AgentPresetRevision {
    revision_id
    preset_id
    revision_no
    payload
    contribution_locks
    revision_digest
}
```

`current_revision_id` 只指向已经通过 Compiler 的不可变 Revision；Draft 不可被 Session、Remote 或 Automation 直接执行。Snapshot、Binding 和 Session 继续引用 Revision/Snapshot，而不是复制一份可编辑 Agent 配置。

`revision_digest` 必须覆盖规范化的 `payload + contribution_locks` Revision envelope；ContributionLock 不能作为未被 digest 覆盖的旁路 JSON。若实现仍保留 payload digest 供编辑器比较，必须另外持久化并校验 lock digest，不能用旧 payload-only digest 冒充最终 Revision 身份。

`AgentPresetSource` 只保留 `official | user` 两种 provenance；`package` 不是 AgentPreset 的来源类型。官方 seed 与用户创建的 Agent 都必须最终生成同一类 User-facing AgentPresetRevision。

`ContributionLock` 是系统生成的依赖事实，不是用户手填的 Package 选择。最小语义为：

```text
ContributionLock {
    source_kind: platform_builtin | plugin_mount | miniapp_active_release | mcp_binding
    source_identity: StableSourceIdentity
    mount_id?: PluginMountId
    miniapp_id?: MiniAppId
    mcp_binding_id?: McpBindingId
    contribution_id: ContributionId
    contract_digest: DigestHex
}
```

`source_identity` 必须是稳定、可审计的来源身份；不得用显示名称、当前版本字符串或注册顺序代替。Snapshot 在此基础上继续锁定本次实际执行所需的：

- 每个选中的 Capability/Skill 的来源和合同事实（Skill 仍是 instruction/workflow，不成为执行器）；
- exact Package/Plugin target 或 MiniApp Release version/digest；
- Tool schema、Context/Resource implementation 和 MCP schema digest；
- Model Route、typed resource binding 和所需 Runtime features；
- initial/on-demand 分组及其他真实执行闭包；
- Snapshot digest 和 provenance。

因此要明确区分：

```text
AgentPresetRevision = 用户意图 + 稳定贡献锁
ResolvedSnapshot    = 某次执行的完整、精确、不可变闭包
AgentSession        = 消费一个 Snapshot 的运行事实
```

#### 15.3.3 Revision 生命周期与 Compiler 不变量

Agent 工作台中的编辑内容先存在于不可执行的 Draft；只有 Save 成功后才创建新的不可变 `AgentPresetRevision`，并原子推进 AgentPreset 的 `current_revision_id`。Preview 和 Test 可以读取 Draft 的编译结果，但不能把 Draft 当作正式 Session 的长期来源。

canonical Compiler 的步骤固定为：

```text
normalize user payload
  → validate catalog capability/skill/resource references
  → resolve default or explicitly requested implementation source
  → generate ContributionLock[]
  → materialize exact capability closure and typed resources
  → calculate revision/snapshot digest and diagnostics
  → persist immutable Revision or return fail-closed diagnostics
```

Compiler 必须满足以下不变量：

1. 同一规范化输入、同一 Catalog generation 和同一资源事实产生同一 Revision/Snapshot 结果；
2. 未发布、不可用、合同不匹配或 owner 不允许的贡献不能被编译成成功结果；
3. Compiler 不安装 Package、不启动 Plugin/MiniApp、不修改 Credential/Runtime，也不自行授予授权；
4. 任何未知字段、未知来源、未知合同或无法比较的变化都 fail closed；
5. Session Open 只读取已保存 Snapshot 并做当前执行可用性检查，不在 Turn 中重新选择 latest 来源；
6. Revision 失败不改变当前 Revision，Snapshot 失败不创建可执行 Session。

#### 15.3.4 明确禁止的字段和能力

以下内容不得进入 AgentPresetRevision 的用户 payload，也不得通过兼容字段继续保留：

```text
preferred_agent_id
agent_preferences
runtime selector
fallback engine
raw plugin dataDir
raw MCP tool definition
plugin install / replace / publish authorization
arbitrary Package or Mount mutation
latest / follow / silent fallback selector
raw `context_policy` / `execution_constraints` / `runtime_budget` JSON
arbitrary `surfaces` or destination policy
```

Runtime、Plugin Config、Credential、KV、Files、MiniApp DB 和发布授权必须留在所属平台域。Agent 可以请求使用它们，但不能借 AgentPreset 获得 owner 权限。若某个 context、execution 或 budget 约束确实有运行时语义，必须由平台定义 typed contract 并由 Compiler 生成/校验，不能把任意 JSON 重新塞回 AgentPreset。

### 15.4 四种模式是创建模板，不是四种持久化类型

用户创建 Agent 时先选择预期能力级别，工作台用一个模板生成初始草稿：

| 模式 | 默认含义 | 首次生成内容 | 明确不做 |
|---|---|---|---|
| 轻量 | 零工具、零外部能力的轻量对话 | 基础身份、指令和模型路由 | 不隐式加入 Workspace、MCP、Plugin 或 MiniApp 能力 |
| 通用 | 官方通用任务能力 | 官方维护的常用 Capability/Skill 种子 | 不自动吸收用户已安装的全部扩展 |
| 全面 | 完整 Coding/工作台基线 | 文件、进程、VCS、Workspace 等官方 Coding 能力 | 不因名称“全面”而自动安装或纳入所有 Plugin/MiniApp |
| 自定义 | 用户明确组合 | 空白或可选种子 + 能力/技能/资源 picker | 不允许用户直接填写内部 ID、Digest 或运行时参数 |

四种模式不是四种 Agent 类型，也不是四张并行数据库表。创建后的 Agent 统一落到 `AgentPreset → AgentPresetRevision → Snapshot` 主链。

- 模板更新只影响之后创建的 Agent；
- 已存在的 Revision 不因官方模板更新而漂移；
- 用户要求把已有 Agent 从一种模式调整到另一种模式时，系统生成可审阅的 capability diff，由用户确认后保存新 Revision，不直接覆盖用户选择；
- Plugin/MiniApp 若未来贡献 `agent_template`，它只能是创建种子，不能复活旧的 `contributes.presets`、自动授予能力或绕过用户确认；N1 首版默认不把该能力作为实现前提。

官方四种模式可以继续由只读的 `agent_preset_templates` seed catalog 提供，但它不是 AgentPreset，也不是 Plugin contribution。AP-3/AP-6 必须把该 catalog 收敛为 `source_kind=official`：删除 `AgentPresetSource::Package`、Package-owned template foreign key 和任何“安装 Package 即获得模板”的路径。未来若确有 Package 模板需求，另立版本化的 `agent_template` contribution 合同，并按本节的“只提供 seed、不授予能力”规则接入。

### 15.5 Agent 工作台的一级交互合同

Agent 工作台的最短用户路径应为：

```text
首页侧边栏 Agent（公共路由 `/agent`）
  → 新建 Agent
  → 选择轻量 / 通用 / 全面 / 自定义
  → 编辑身份、指令、模型、能力、技能和资源
  → 查看能力来源与可用性
  → 保存并试用
  → 在新会话中继续使用
```

首页侧边栏不再用“设定”作为 Agent 入口；通用系统设置（例如 Runtime、网络或外观）仍可由全局设置入口提供，但不与 Agent 工作台共用名称、页面或数据模型。

页面只需要展示：

- Agent 名称、用途和当前模式种子；
- 按任务分组的能力和技能 picker；
- 工作区、知识库、MCP 连接和其他连接器等 typed resource picker；
- 模型路由和 starter prompts；
- 能力来源、版本状态、缺失原因和影响提示；
- 保存、试用、复制/Fork、删除等用户动作。

以下内容默认放入折叠的技术详情或导出诊断，不作为普通编辑项：

- Revision、Snapshot、Digest、ContributionLock；
- Package/Mount/Release provenance；
- Runtime protocol、Host generation 和内部错误上下文。

不可用能力必须在工作台中显示为可解释状态，并提供“补充资源”“启用来源”“复制为新 Revision”或“在新会话继续”等明确动作；不得用空能力、第一方 fallback 或静默降级制造成功感。

### 15.6 Agent 与非 Agent 消费者的统一执行边界

Agent 使用路径固定为：

```text
Agent 工作台 Save/Preview
        → canonical AgentPreset Compiler
        → immutable AgentPresetRevision
        → ResolvedSnapshot
        → AgentSession / AgentBinding
```

Gateway、Remote、Automation、UI/业务域和 MiniApp Service 不需要伪造一个 AgentPreset。它们直接使用同一 Materializer、Catalog、typed resource resolver 和 exact operation lock：

```text
非 Agent 操作
        → consumer-specific Resolver
        → exact OperationLock
        → owning runtime/service
```

统一主链的含义是共享能力合同和来源锁，而不是强迫所有消费者创建 AgentSession：

1. AgentSession 只属于 Agent 对话和明确的 Agent 运行场景；
2. 一次性 Gateway、Knowledge、Automation 或 MiniApp 操作可以没有 Session，但必须记录自己的 exact lock；
3. 消费者不得直接引用 Plugin/Browser/Computer 的具体实现；
4. AgentPreset 不拥有 Knowledge、Browser、Plugin、MiniApp 业务数据，只绑定正式资源；
5. Capability 缺失、合同变化、Credential/Resource 不可用时返回 typed failure，不能由消费者各自实现 fallback。

### 15.7 Plugin/MiniApp 变化对 Agent 的影响

| 变化 | AgentPreset/Revision 行为 | 新使用行为 | 禁止行为 |
|---|---|---|---|
| Compatible Replace | 不修改既有 Revision；新 Snapshot 按稳定来源和合同重新解析并记录 exact target | 能力可继续使用，或按 Snapshot exact lock 给出 unavailable | 不静默改写旧 Revision，不跨来源替换 |
| Breaking Replace | 派生受影响 Agent Revision、Binding、Automation、Remote 清单 | 旧 Revision 返回 typed contract mismatch，用户显式修复或 Fork | 不自动迁移、不自动扩大能力、不静默 fallback |
| Plugin Disable/Uninstall | Revision 和历史 Session 保留 | 新 Session/新操作返回 capability unavailable | 不把其他 Package 当作替代实现 |
| MiniApp 非 Active Release | 不进入正式 Agent Catalog | 不能被 Agent 选择或调用 | Ready/Previous/测试 Release 不伪装成 Active |
| Active MiniApp Release 变更 | 以 `miniapp_id + contribution_id + contract_digest` 重新解析并记录实际 Release digest | 合同兼容时继续；不兼容时显式失败 | 不让 Agent 改写 MiniApp Publish/Service 状态 |
| Candidate/Source/Build 变化 | 不影响现有 Agent Revision | 只有 Apply/Publish 后的正式贡献才可能产生影响 | Candidate Test 结果不能直接满足 Agent 正式执行资格 |

已有 Session 的 Snapshot 不因全局 Catalog 变化而被改写。若其精确执行来源不再可用，必须返回明确的 Snapshot/Executor unavailable；用户可以显式 Fork 或创建新 Session，但不能在后台换实现。

### 15.8 Canonical API、应用服务与旧模型删除

Agent 相关公共 API 统一收敛为：

```text
/api/agent-presets/*
/api/agent-preset-templates/*   # 只读官方创建种子
/api/agent-sessions/*
/api/agent-bindings/*
```

API 保留 `agent-presets` 作为稳定机器资源名，不代表 UI 必须显示“Agent 设定”；用户界面统一使用 Agent/Agent 工作台。

平台 Capability Catalog 使用自己的通用资源 API；Plugin、MiniApp、Runtime 和 Credential 继续使用各自 owner API。前端和外部调用方应通过 application service 完成高层动作：

- 创建 Agent 和初始模板草稿；
- 保存/复制/Fork AgentPresetRevision；
- Preview、Save、Test；
- 使用 Agent、创建 AgentSession、应用 AgentBinding；
- 查询能力来源、可用性和影响清单。

`from-template` 只允许引用官方 seed catalog；它创建的是 AgentPreset Draft，不直接创建可执行 Snapshot，也不从 Package/Plugin 自动继承能力或授权。

客户端不得提交 Snapshot digest、内部 Revision ID、Mount ID、完整 Binding、内部 owner ID 或 canonical JSON 来驱动执行。需要展示时由服务端返回面向产品的 DTO，技术详情只读。

在 AP-6 中必须删除而不是保留兼容别名的旧形态：

- `/api/presets` 及其路由、DTO、handler；
- `/api/extensions/presets` 及其 Extension/Plugin preset adapter；
- `PresetService`、旧 Preset resolver/compiler 和旧 `ResolvedPresetSnapshot` 链；
- 旧 `presets` 表/迁移/fixture（若仍存在）、旧 preset 子表及只写不读的 preset projection；
- Extension/Plugin 的 `contributes.presets`、`ExtPreset`、`ResolvedPreset` 等旧贡献模型；
- 任何把 `preferred_agent_id`、`agent_preferences`、`system_prompt`、fallback 或 runtime selector 当作旧兼容字段继续读写的路径。

Fresh-v4 采用 clean cut：

- 不做旧 `/api/presets` 与新 `/api/agent-presets` 的双读双写；
- 不在运行时按版本猜测旧数据；
- 旧 v3 数据根按归档/清理规则处理，不进入新 Agent 主链；
- 代码、路由、数据库、生成 schema 和文档中的生产可达旧路径必须达到 0；
- 真实用户数据若需要保留，必须由 owning domain 提供显式、一次性的导出/归档流程，不把兼容负担塞回 AgentPreset。

#### 15.8.1 当前仓库的实施切片基线

下面是 AP 施工前已经确认的引用面，作用是领取任务和做 residual 扫描，不替代实现后的 canonical source：

| 层 | 当前主要位置 | AP 处置 |
|---|---|---|
| Contract/Schema | `crates/backend/nomifun-agent-contracts/src/preset.rs`、`crates/backend/nomifun-api-types/src/preset.rs`、`crates/backend/nomifun-agent-contracts/schema/0001_fresh_v4.sql` | 按 AP-2 收敛 payload、ContributionLock、模板 seed 和最小索引 |
| Generated inventory | `crates/backend/nomifun-agent-contracts/contracts/generated/*`、`contracts/presets/*` | 重新生成 Agent API、template API、禁止旧路由和 schema inventory |
| Compiler/Control Plane | `crates/backend/nomifun-agent-control-plane/src/compiler.rs`、`service.rs`、`routes.rs` | 统一 Preview/Save/Test/Session application service，不让前端拼 Snapshot |
| Platform/Session | `crates/backend/nomifun-agent-platform/src/platform.rs`、`crates/backend/nomifun-agent-session/src/*`、`crates/backend/nomifun-v4-root/src/database.rs` | 接入唯一 Revision/Snapshot/Binding/Session 主链，删除重复投影 |
| 新 Agent UI | `ui/src/renderer/pages/agentSettings/*`、`ui/src/renderer/pages/agentSession/CanonicalAgentRoutes.tsx`、`ui/src/renderer/components/layout/Router.tsx` | 挪到公共 `/agent`，保留产品语言和 picker，技术详情折叠 |
| 旧 Agent UI 入口 | `ui/src/renderer/pages/settings/PresetSettings/*`、`ui/src/renderer/pages/settings/AgentSettings/*`、`ui/src/renderer/components/settings/SettingsModal/contents/AgentModalContent.tsx` | 从 Settings/Modal 删除 Agent authoring surface；`/presets`、`/settings/agent-presets`、`/settings/agent` 只做限期迁移跳转 |
| 旧 Preset 主链 | `crates/backend/nomifun-preset/*`、`crates/backend/nomifun-agent-execution/*`、Guid/Conversation/Cron/Companion/Creative Studio/Extension/Gateway 等消费者 | 逐个迁移到 Agent application service 后删除旧路由、服务、DTO 和 fallback |

本表中的路径是当前扫描结果，不构成“只改这些文件”的豁免。AP-6 必须以全仓生产引用扫描、路由测试、Schema inventory 和真实消费者行为为最终判断。

### 15.9 AgentPreset 实施 TODO（AP-0～AP-7）

以下任务是 06 Plugin/MiniApp 开发的前置项目。每项都必须有对应的合同、实施产物和行为验证证据；代码、Schema、UI 和测试按任务适用，但只完成文档或只添加类型不能视为完成。

#### AP-0：产品与领域边界冻结

交付：

- 冻结“Agent 工作台”一级入口和中文术语；
- 发布 Package、Plugin、MiniApp、Capability、Skill、AgentPreset、Revision、Snapshot、Binding、Session、Runtime 的 owner 矩阵；
- 明确 Plugin/MiniApp 是平台能力供给层，不是 Agent 子系统；
- 标记 Runtime/Plugin Config/Credential/KV/`dataDir` 不属于 Agent 编辑内容；
- 选定 AP-1/AP-4 使用的现有 first-party shared Capability、non-Agent-only Capability 和真实非 Agent consumer；
- 把本节和 06 的依赖关系写入实现 Prompt、Schema 目录和发布清单。

通过条件：产品导航、API 命名、数据库 owner 和文档中不存在两个含义相同的“设定”；任何新设计都能回答“谁拥有它、哪些消费者使用它”。

#### AP-1：平台级 Capability 消费模型

交付：

- 通用 Capability Catalog entry、Contribution、provenance、availability 和 `supported_surfaces/consumers` 合同；
- Plugin、MiniApp、Built-in、MCP contribution 的统一 materialization 入口；
- Candidate/未发布 Release 不得进入正式 Catalog 的 admission 规则；
- Agent、至少一个非 Agent 消费者使用同一 Catalog/resolver 的代表实现；同时用一个明确标记为 non-Agent-only 的 Capability 验证 Agent 工作台会正确过滤它。上述代表实现可以使用现有 first-party Capability，不以 06 Plugin/MiniApp 代码先行交付为前提。

通过条件：同一真实 Capability 可以被 Agent 和一个非 Agent 消费者使用，且 non-Agent-only Capability 不会出现在 Agent picker；没有因为 AgentPreset 存在而转移 owner 或复制一套 Agent 专属 Catalog。

#### AP-2：AgentPreset Revision、ContributionLock 与 Compiler

交付：

- `AgentPresetRevisionPayload` 的 canonical schema；
- 系统生成的 `ContributionLock`、Revision digest 和 Snapshot envelope；
- Revision digest 覆盖 payload 与 ContributionLock，Snapshot digest 覆盖实际执行闭包；
- Preview、Save、Test、Session Open 共用一个纯函数 Compiler；
- typed resource binding、MCP schema/provenance 和 Role Provider lock 接入同一解析边界；
- 删除未被真实消费者读取的 `required/exposure/destination_constraints/context_budget_override/tool_budget_override/config` 等旧选择字段、重复的 preset capability/model/skill 子表和 raw JSON runtime knobs；保留的字段必须有明确执行语义；
- 禁止字段清单和未知字段 fail-closed 校验。

通过条件：同一输入在 Preview/Save/Test/Session Open 得到相同结果；用户不能通过 API 或 UI 直接伪造 Snapshot、Mount、Digest 或 fallback。

#### AP-3：四种模式与 Agent 工作台

交付：

- 轻量、通用、全面、自定义四个创建模板；
- 模式只是 seed，不产生四种持久化类型；
- 首页一级入口（公共路由 `/agent`；Session 使用 `/agent-sessions/:id`）、能力/技能/资源 picker、来源状态、保存和试用流程；
- 从 `settings/AgentSettings`、`SettingsModal` 和 `/settings/agent-presets` 中移除 Agent authoring surface；旧深层链接最多保留一次性迁移跳转，不形成长期第二入口；
- 模式转换的显式 diff/确认和模板更新不漂移既有 Revision；
- 技术 Inspector 与产品编辑表单分离。

通过条件：新用户不需要进入深层设置即可完成“建 Agent → 配能力 → 试用”；全面模式不会自动加入全部 Plugin/MiniApp；轻量模式不会暗含工具或外部能力。

#### AP-4：Agent 与非 Agent 消费者接入

交付：

- AgentSession、AgentBinding、Remote、Automation、Gateway、Knowledge/MiniApp 等实际消费者的统一 application service；
- 有 Session 与无 Session 的 exact lock 两条清晰调用路径；
- 前端不提交内部执行锁，后端负责生成 Revision/Snapshot/OperationLock；
- 真实消费者的高层 API 和 typed failure。

通过条件：至少一个真实 Agent 流程和一个真实非 Agent 流程通过同一 Capability materialization；消费者代码不认识具体 Plugin/MiniApp implementation。

#### AP-5：来源、生命周期和影响处理

交付：

- `mount_id/miniapp_id + contribution_id + contract_digest` 的稳定绑定；
- Compatible Replace、Breaking Replace、Disable、Uninstall、MiniApp Active Release 变化的 impact diff；
- Agent 工作台的来源、可用性、影响清单、Retry/Switch/Restore/Fork 体验；
- Candidate、Test、正式执行之间的边界和 typed errors。

通过条件：合同变化不会自动改写旧 Revision 或静默替换来源；历史 Session 保留事实，新使用失败可解释且不 fallback。

#### AP-6：旧 API 与旧数据模型 clean cut

交付：

- 旧 `/api/presets`、`PresetService`、旧 DTO/resolver、旧表/迁移/fixture（若仍存在）和旧 Extension Preset contribution 的生产可达性清单；
- 删除旧路由、双写、兼容 alias、旧 fallback 和旧 projection；
- Fresh-v4 schema、generated inventory、主导航和代码引用同步更新；
- 删除设置页中的 Agent authoring 入口和 `SettingsModal` 内的 Agent 入口；`/presets`、`/settings/agent-presets`、`/settings/agent` 只允许有明确期限的一次性迁移跳转，完成迁移后必须移除；`/settings/execution-engines` 只保留 Runtime Manager，不再承载 Agent authoring；
- 保留官方 `agent_preset_templates` 只读 seed catalog，移除 Package-owned template/source 分支；删除 `AgentPresetSource::Package` 和重复的 preset capability/model/skill/resource projection；
- v3 数据归档/清理说明和必要的显式导出入口。

通过条件：生产路由、应用服务、数据库读写和前端入口中旧 Preset 主链为 0；新 Agent 主链不依赖旧数据猜测或隐式转换。

#### AP-7：验证、发布和 06 admission gate

交付：

- Contract、Compiler、Catalog、Consumer、Impact、No-fallback 和旧路径 residual 测试；
- 至少一个真实 Agent 消费者与一个真实非 Agent 消费者的端到端证据；
- API/Schema/generated inventory、README、05/06 交叉引用和实施 Prompt 更新；
- 06 N1-0 的 admission checklist 和阻断状态。

通过条件：下列门禁全部为真，才允许开始 06 的 Plugin/MiniApp 代码：

```text
AP-0..AP-6 completed
AP-7 admission evidence signed
AgentPreset API/Revision/Snapshot/Binding frozen
one real Agent consumer + one real non-Agent consumer
non-Agent-only capability filtered from Agent picker
ContributionLock and impact diff representative tests passed
legacy /api/presets production reachability = 0
Fresh-v4 schema/generated inventory updated
Plugin/MiniApp contribution no longer depends on old Preset
```

### 15.10 实施顺序与 06 的明确依赖

AgentPreset 的实际施工顺序固定为：

```text
05 P1-R0～P1-R3（Role/Provider 基础合同）
  →
AP-0 术语/owner/入口冻结
  → AP-1 通用 Capability Catalog
  → AP-2 AgentPreset Compiler/Revision/Snapshot
  → AP-3 Agent 工作台与四种模板
  → AP-4 Agent + 非 Agent 消费者
  → AP-5 provenance/impact/lifecycle
  → AP-6 删除旧 API/模型
  → AP-7 验证与 admission gate
  → 06 N1-0 Plugin/MiniApp 机器合同冻结
  → 06 N1-1 及后续实现
```

06 可以在 AP 阶段继续做设计审阅、交叉引用和合同修订，但不能提前实现 Node Host、Plugin Loader、MiniApp Release 或任何以旧 Preset 为入口的代码。若 AP-1 或 AP-2 尚未完成，06 中的 Catalog/Agent integration 章节只能视为未授权的设计草案。

建议的消费者迁移顺序是：先完成 Agent 工作台与 AgentSession 的 canonical application service，再迁移普通 Conversation/Guid/Chat；随后迁移 Cron/Automation/Remote，最后迁移 Companion、Creative Studio、Extension 和其他低频入口。每迁移一类消费者都要删除其旧 `PresetService` 注入，直到 AP-6 的生产可达性为 0；不能用“新旧同时调用一段时间”的长期双主链替代迁移完成。

### 15.11 本版默认锁定与仍可拍板的事项

为减少后续反复，本版默认锁定以下决策：

1. 对用户只保留一个 Agent 工作台一级入口；
2. `AgentPreset` 是内部统一聚合根，不再保留旧 Preset 产品和旧数据模型；
3. 四种模式是创建模板，不是四种 Agent 类型；
4. Plugin/MiniApp 先向平台 Capability Catalog 供给能力，再由 Agent 或其他消费者选择；
5. Runtime/Plugin/MiniApp 管理不进入 AgentPreset payload；
6. 旧 `/api/presets`、旧服务和旧贡献模型 clean cut，不做双读写和静默迁移；
7. 缺失、合同变化和来源失效都 typed fail，不使用隐式 fallback。

在 AP-0 评审时仍可以明确拍板、但不应阻塞总体方向的三个产品细节是：

- **公开名称：**建议导航显示“Agent”，页面标题显示“Agent 工作台”；
- **实现来源选择：**建议只在能力详情中提供高级显式动作，不建设独立 Provider/Runtime 设定页；
- **Plugin 模板：**建议 N1 首版先不开放 `agent_template` contribution，后续只作为不授予能力的创建种子。

除非 AP-0 评审明确否决上述默认值，否则实施者应按本节合同继续，不得重新引入“旧设定 + Agent 设定”双轨。
