# Agent Capability Platform v2 全局闭合 TODO

> 盘点日期：2026-09-02
> 基线分支：`rf/agent-capability-platform-v2`
> 代码基线 HEAD：`f97b281c669d9298413008921a2d65407473ffa9`
> 当前结论：`C8-WIN-PRE`、`HP-1`、`C8-MA`、`C8-MERGE`、`C9` 和发布均未完成。

本文档是当前重构的全局执行台账。稳定 TODO ID 不因排序、负责人或提交变化而复用；
完成后将状态改为 `closed` 并补充 evidence，而不是删除条目。本文只统计可交付工作包，
不把源码 grep 命中数、历史 residual 数或测试数量当作任务数量。

本轮 Wave 3/4、AgentSession ServiceKey 和 VCS Push 已分别形成定向测试通过的提交，
并与远端 macOS Knowledge 修复通过普通 merge 合流。它们关闭了若干实施子步骤，但尚未
满足对应稳定 TODO 的全部完成定义，因此仍按 `open` 或 `blocked` 管理。只有 production
owner、持久化、真实消费者、中央组合和指定 evidence 全部满足后，才能关闭稳定 TODO ID。

任何 provider API key、安装令牌或 Sidecar 凭据都不得写入本文档、源码、测试 fixture、
Git 提交或 Agent prompt。真实验证只能通过现有凭据存储或一次性环境注入读取 secret。

## 已关闭基线，不得重复列为未完成

| Commit | 已关闭范围 |
| --- | --- |
| `745fabfa` | Fresh-v4 binding-backed `knowledge.search` / `knowledge.read` production owner |
| `280841b3` | Agent Preset 编辑器选择真实 KnowledgeBase 并生成 canonical resource binding |
| `5d691824` | Knowledge search/read root-anchored、逐组件 no-follow 文件访问与 Windows fault tests |

上述提交已经使真实 canonical dev root 上的 `bun run dev` 能正常启动 Desktop，不再出现
storage generation 崩溃。它们不代表 `knowledge.write`、整个 Wave 1、C8 或发布完成。
曾经实现但未提交的 `knowledge.write` 原型不属于已完成工作。

## 本轮已落盘进度

| Commit | 已完成子步骤 | 仍未关闭 |
| --- | --- | --- |
| `3f835174` | 同一 `AgentPlatform` generation 提供 canonical Session command/query ServiceKey；Wave 5 manifest 声明精确依赖并透传 `DeclaredServiceView` | `INF-005`：AgentExecution、Cron、AutoWork、IDMM、Channel、Remote 等真实消费者尚未迁移 |
| `765d1953` | Wave 4 的 11 个 action-specific DTO、effect route、`succeeded/failed/uncertain/reconciled` receipt 状态机及 20 个合同测试 | `W4-001`：领域 repository、事务 outbox/inbox、reconciliation persistence 和真实 owner 尚未实现 |
| `d1acccf6` | Wave 3 的 19 个 typed operation/outcome 映射；其中 15 个合同冻结，13 个测试通过 | `W3-001`：`workshop.director` 与 3 个 Office edit mutation contract 仍显式 fail-closed |
| `8aade375` | `vcs.push` 接入 Wave 2 production host；真实 local/file bare remote、failure replay、同 key no-repeat 共 11 个测试 | `W2-001`：SSH/HTTPS credential authority、网络 remote live evidence 与显式 reconcile 尚未完成 |
| `efbcb598`、`23e039ff`、`1a547f3a`、`f46cc017` | macOS system alias/target guard、replaced-root fault test 与 arm64 原生记录已合流；anchored/bound/app tests 为 `9 + 4 + 4` | `TST-003`：macOS x64 与 Linux 仍待复验；`SCN-012`：arm64 helper 因无 Universal x64 slice、真实 Sidecar 和运行资源而 FAIL |

本轮额外验证：`cargo check -p nomifun-app`、SQLite AgentPlatform restart、Fresh-v4
canonical host 双启动、Windows anchored Knowledge 7 项测试均通过。完整 C8 未运行。

## 汇总口径

### Capability inventory

| Wave | Inventory capability | Action-bearing | 已有真实 action owner | 缺失 action owner | Non-action / contribution |
| --- | ---: | ---: | ---: | ---: | ---: |
| Wave 1 | 25 | 14 | 8 | 6 | 11 |
| Wave 2 | 41 | 27 | 12 | 15 | 14 |
| Wave 3 | 19 | 19 | 0 | 19 | 0 |
| Wave 4 | 22 | 11 | 0 | 11 | 11 |
| Wave 5 | 21 | 10 | 0 | 10 | 11 |
| **总计** | **128** | **81** | **20** | **61** | **47** |

已有的 20 个真实 action owner 是：

- Wave 1：`web.fetch`、`knowledge.search`、`knowledge.read`、五个
  `memory.project.*` / `memory.companion.*` mutation action，共 8 个。
- Wave 2：六个 `fs.*` action、`vcs.status/diff/stage/commit/push` 和
  `process.exec`，共 12 个。

`web.fetch` 已有执行 owner，但其 ExternalTransmit result replay/receipt 尚未闭合；
`vcs.push` 已有 local/file remote owner，但网络 credential authority 尚未闭合。因此
“已有 owner”不等于“整个 capability 已完成”。47 个 non-action capability 也不按
action owner 缺口计数，其真实 context、middleware、scheduler、transport、resource
provider 和平台 availability 分别由下方工作包核查。

最近一次完整 Windows C8 是历史 clean SHA `b849e2ac...` 上的诊断运行：
`source=556`、`blocking=0`、`contract-allowed=26`、`deferred-to-C9=530`。
它早于当前 HEAD，且 owner coverage 与 workspace test 均失败，不能作为当前候选 evidence。
这些 residual 数字是源码分类结果，不是 556 个 TODO。

### 工作包数量

| 分区 | open | blocked | external | pending-validation | 合计 |
| --- | ---: | ---: | ---: | ---: | ---: |
| 全局基础设施 `INF` | 4 | 2 | 0 | 0 | 6 |
| Wave 1 `W1` | 5 | 5 | 0 | 0 | 10 |
| Wave 2 `W2` | 7 | 0 | 0 | 0 | 7 |
| Wave 3 `W3` | 6 | 0 | 0 | 0 | 6 |
| Wave 4 `W4` | 6 | 0 | 0 | 0 | 6 |
| Wave 5 `W5` | 0 | 6 | 0 | 0 | 6 |
| 旧组合图 `LEG` | 0 | 8 | 0 | 0 | 8 |
| Sidecar / Native `SCN` | 1 | 1 | 13 | 0 | 15 |
| 测试 / Gate `TST` | 5 | 5 | 0 | 2 | 12 |
| C9 / Release `REL` | 0 | 6 | 2 | 0 | 8 |
| **总计** | **34** | **33** | **15** | **2** | **84** |

状态定义：

- `open`：当前仓库与 Windows 主机可以直接实施。
- `blocked`：必须先完成列出的合同、owner、schema 或组合依赖。
- `external`：需要仓库外源码/制品、真实硬件、签名环境或凭据。
- `pending-validation`：主要实现已存在，但指定的真实环境或人工 evidence 尚未取得。
- `closed`：完成定义与最小验证均满足，并记录了 clean SHA/evidence。

## 推荐并行批次

| 批次 | 可并行工作 | 冲突控制 |
| --- | --- | --- |
| P0 合同冻结 | `INF-001`、`INF-003`、`INF-004`、`INF-005` 可由独立 Agent 先做只读设计审计，再由各自唯一写 owner 落盘 | `agent-contracts` generator、Fresh-v4 migration 和 `Cargo.lock` 各保持单一写 owner |
| P1 领域实现 | Wave 2 SSH/MCP/Connector；Wave 3 四个业务 crate；Wave 4 四个业务 crate；Sidecar 源码流可并行 | 领域 Agent 先只改本领域 crate；不并发编辑 app composition |
| P2 中央接线 | 一个 integration Agent 串行合入 Wave 1～5 host ports、资源 catalog 和 provider ports | `agent_platform_host.rs`、`agent_wave2_host.rs`、`agent_wave4_host.rs` 同时只允许一个写 owner |
| P3 消费者迁移 | Wave 5 automation 与 Wave 4 identity/channel 可在不同领域 crate 并行；最后统一改 app/router | `services.rs`、`router/state.rs`、`GatewayDeps`、legacy route 由同一个 demolition owner 串行处理 |
| P4 验证 | 只读 audit、文档核查、native 主机准备可并行；Cargo 与最终 Gate 按 tuple 去重 | 不让多个 Agent 同时跑 workspace Cargo；不在单项失败后立即触发跨机 recheck |

提高效率的关键不是让多个 Agent 同时修改中央文件，而是把“领域 crate 实现”和“中央
composition 合流”分成两个阶段。`Cargo.lock`、Gate 常量、生成 manifest 和 release
digest 也必须由单一 coordinator 在批次末统一刷新。

## 多机分工

### 分工原则

1. 主机是中央 integration owner，负责合同最终落盘、Fresh-v4 schema、app composition、
   shared dependency lock、Gate、manifest、digest、旧图拆除和最终 Windows Gate。
2. 机器 2 只领取下表明确允许的领域目录或原生验证任务。代码任务必须使用独立 clone /
   worktree 和独立分支，基于主机提供的 clean base SHA；不得直接假设能看到主机未提交
   的 Wave 3/4 WIP。
3. 机器 2 不修改中央文件。需要中央接线时，只提交领域 crate、测试和一份接线说明，由
   主机 integration owner 串行完成。
4. 同一稳定 TODO ID 同一时间只有一个写 owner。其他 Agent 可以只读审计，但不能提交
   竞争实现。
5. 机器 2 的每次回传必须包含 base SHA、commit SHA、changed paths、已运行命令、
   PASS/FAIL、未运行项和 blocker。禁止 force-push、合并生成物猜测或 synthetic PASS。
6. provider credential、installation token、签名 secret 和 Sidecar secret 只保留在
   执行主机的 secret store / environment，不进入 Git 交接。

### 可交给机器 2 的任务

当前首个分配为 `M2-W2-SSH / W2-002`，Git 分支固定为
`rf/m2-w2-ssh-owner`。机器 2 在该 lane 完成前独占
`crates/backend/nomifun-ssh/**` 与 `crates/shared/nomi-ssh/**`；主机继续 Wave 5、
中央接线、旧图拆除及非 SSH 工作，不等待机器 2 才推进。精确 base SHA、启动命令、
完成定义和回传格式记录在机器 2 分支的专用启动 Prompt。

| 机器 2 lane | 可领取 TODO | 允许修改范围 | 交付边界 |
| --- | --- | --- | --- |
| M2-W2-SSH | `W2-002` | `crates/backend/nomifun-ssh/**`、`crates/shared/nomi-ssh/**` 及其专属测试；如需 domain DTO，只提交建议或等待主机分配明确文件 | 完成 SSH owner primitive、fault tests 和接线说明；`agent_wave2_host.rs` 由主机接入 |
| M2-W2-MCP | `W2-003`、`W2-004` | `crates/backend/nomifun-mcp/**`、Connector 专属新模块/测试 | 完成 MCP/Connector owner primitive 与 live-test harness；中央 provider/resource binding 由主机接入 |
| M2-W2-BROWSER | `W2-005` | Browser 专属 crate 与平台 adapter 测试 | 完成 Browser action owner primitive和目标平台 availability；不得修改中央 host/Gate |
| M2-W2-COMPUTER | `W2-006` | Computer/A11y 专属 crate 与 OS adapter 测试 | 完成 Windows/macOS adapter 和 exact-unavailable contract；权限人工 evidence 单独回传 |
| M2-W3-CREATION | `W3-003` | `crates/backend/nomifun-creation/**` | 基于已冻结 DTO/resource contract 实现 5 个 owner；未冻结前只做只读审计 |
| M2-W3-WORKSHOP | `W3-004` | `crates/backend/nomifun-workshop/**` | 实现 6 个 owner 与领域测试；Canvas schema/central host 不在此 lane 修改 |
| M2-W3-OFFICE | `W3-005` | `crates/backend/nomifun-office/**` | 实现 4 个 owner、artifact/preview tests 和接线说明 |
| M2-W3-MINIAPP | `W3-006` | `crates/backend/nomifun-miniapp/**` | 实现 4 个 owner、publish/serve fault tests 和接线说明 |
| M2-W4-CHANNEL | `W4-002` | `crates/backend/nomifun-channel/**` | 实现 Channel domain owner/outbox primitive；中央 AgentSession/Remote 接线留给主机 |
| M2-W4-COMPANION | `W4-003` | `crates/backend/nomifun-companion/**` | 实现 Companion domain owner与领域测试；不得重新引入 Conversation/Nomi 依赖 |
| M2-W4-CUSTOMER | `W4-004` | `crates/backend/nomifun-customer-service/**` | 实现 Customer Service owner、receipt/outbox tests 和接线说明 |
| M2-W4-ROBOT | `W4-005` | `crates/backend/nomifun-robot/**` | 实现 Robot owner primitive；无真实设备时只提交 fail-closed tests，不生成硬件 PASS |
| M2-W4-NOTIFY | `W4-006` | Notification 专属领域模块/测试 | 实现 webhook/desktop delivery primitive；中央 outbox migration 由主机落盘 |
| M2-SIDECAR | `SCN-001`、`SCN-003`～`SCN-010` | 独立 Codex fork/patch repository；主仓仅在主机明确分配后更新 `vendor/codex-runtime/**` | 回传可审计 patch source、upstream base、build 命令、binary/metadata digest；普通 codex binary 不算完成 |
| M2-MAC-ARM | `SCN-012`、`TST-003`、`REL-003` | 真 Apple Silicon clone、repo-local evidence 目录 | 只在 HP-1 后执行 whole-candidate native Gate；当前可做 engineering validation，但不得写 C8-MA PASS |
| M2-MAC-X64 | `SCN-013`、`TST-003` | 真 Intel Mac clone、repo-local evidence 目录 | 只在 HP-2 后执行 x64 native Gate；Rosetta 不可代验 |
| M2-LINUX-DESKTOP | `SCN-014` | 真 Linux Desktop x64 clone、repo-local evidence 目录 | 执行 target-specific package/full Coding/Browser/lifecycle/fault evidence |
| M2-LINUX-HEADLESS | `SCN-015` | 真 Linux Headless x64 clone、repo-local evidence 目录 | 执行 target-specific install/full Coding/lifecycle/fault；Browser/Computer 必须 exact-unavailable |
| M2-MANUAL-UI | `INF-006`、`TST-004` | 不改代码，或仅在主机明确分配的 UI 测试文件中修复 | 回传逐步人工结果、截图/日志和准确 commit；不得把人工观察写成未执行自动 Gate PASS |
| M2-READONLY-AUDIT | 任一仍 open/blocked ID | 只读 | 输出缺口、风险、推荐测试和冲突文件；不改变 TODO 状态 |

机器 2 不应领取 `W3-001`、`W3-002`、`W4-001` 的最终合同/schema/persistence 落盘。
这些中央合同已有主机提交且仍存在明确未闭合项；机器 2 只能基于主机给出的 clean
checkpoint 接手表中列出的领域 crate，不能自行改写合同或把局部 owner 判为全局 closed。

### 仅主机所有的中央任务

| 主机 lane | TODO | 中央所有权 |
| --- | --- | --- |
| HOST-CONTRACT | `INF-001`、`INF-003`、`W3-001` | canonical contract、schema refs、generated envelopes 和 action DTO/outcome 最终版本 |
| HOST-SCHEMA | `INF-004`、`W3-002`、`W4-001` | Fresh-v4 baseline/evolution、migration ordering、domain resource/outbox tables、schema digest |
| HOST-SESSION | `INF-005`、`W5-001`～`W5-006` | 同一 AgentPlatform 的 Session ServiceKey、automation consumers 和 Remote product composition |
| HOST-PROVIDER | `INF-002`、Wave 1 provider tasks | production provider route/credential composition 和 app host 注入 |
| HOST-WAVE-INTEGRATION | 所有 Wave 中央接线 | Wave 1～5 host-port composition、fallback removal、owner coverage |
| HOST-LEGACY | `LEG-001`～`LEG-008` | `AppServices`、router state、GatewayDeps、legacy routes/DTO 和 Nomi production reachability 拆除 |
| HOST-WINDOWS | `SCN-002`、`SCN-011`、`TST-001`、`TST-002`、`TST-005`～`TST-012` | Runtime host contract、Windows full candidate、workspace test 去重、UI baseline、Remote hang、all-scene/fault evidence |
| HOST-RELEASE | `REL-001`、`REL-002`、`REL-004`～`REL-008` | tuple freeze、manifest/digest、HP coordination、C8-MERGE、D-027、C9、C10、C11 |

以下路径默认只有主机 integration owner 可以编辑；机器 2 如确有必要，必须先取得
逐文件转让，且主机在转让期间停止编辑该文件：

- `Cargo.toml`、`Cargo.lock`
- `crates/backend/nomifun-agent-contracts/**`
- `crates/backend/nomifun-v4-root/**`
- `crates/backend/nomifun-agent-platform/src/lib.rs`
- `crates/backend/nomifun-agent-platform/src/platform.rs`
- `crates/backend/nomifun-agent-platform/src/session_services.rs`
- `crates/backend/nomifun-app/src/services.rs`
- `crates/backend/nomifun-app/src/router/state.rs`
- `crates/backend/nomifun-app/src/router/routes.rs`
- `crates/backend/nomifun-app/src/router/agent_platform_host.rs`
- `crates/backend/nomifun-app/src/router/agent_wave2_host.rs`
- `crates/backend/nomifun-app/src/router/agent_wave4_host.rs`
- `crates/backend/nomifun-gateway/src/deps.rs` 及 Gateway central composition
- `scripts/gate-agent-v2.mjs`
- `docs/specs/2026-08-28-agent-capability-platform-v2/*MANIFEST*.json`
- `docs/specs/2026-08-28-agent-capability-platform-v2/*CLOSURE*.json`
- `crates/backend/nomifun-agent-contracts/contracts/generated/**`
- `crates/backend/nomifun-agent-contracts/contracts/runtime/**`
- `vendor/codex-runtime/release-input.json`

### 多机合流准入

机器 2 的提交只有同时满足以下条件才进入主机合流队列：

- base SHA 是主机书面提供的 clean checkpoint，或回传中明确列出偏差；
- changed paths 全部位于分配范围，没有 `Cargo.lock`、中央 host、Gate 或生成 manifest
  的顺带修改；
- 提交包含与风险匹配的定向测试，不用 mock/synthetic success 代替 production owner；
- `git diff --check` 通过，未提交 secret、绝对本机路径或 build output；
- 失败和未执行项按真实状态记录，不把 cross-compile、Rosetta、VM 或人工观察升级为
  native Gate PASS；
- 主机 cherry-pick/merge 后重新运行受影响的中央 integration tests，机器 2 的局部
  PASS 不自动关闭 TODO。

## 全局基础设施

| ID | 状态 | 范围 / 目标 | 依赖 | 完成定义 | 最小验证命令 | 用户 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- |
| INF-001 | blocked | 冻结 non-chat exact model route：Embedding、Rerank、one-shot completion，并明确 Web Search 是否属于 provider route | 当前持久化和 Snapshot 只完整表达 `agent_chat` | route task、provider/model/config revision 和 digest 被 Snapshot exact pin；无通用字符串猜测或回注旧 provider graph | `cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check` | 需要产品确认 Web Search provider 选择；不要提供明文 key |
| INF-002 | blocked | 提供 Fresh-v4 窄 `EmbeddingPort`、`RerankPort`、`OneShotChatPort`，按需要增加 `WebSearchPort` | INF-001 | production adapter 只经 canonical route/credential authority；provider unavailable typed fail-closed；不依赖 `AppServices` | `cargo test --locked -p nomifun-chat-model-broker production:: --lib` | live 验证需要已配置 provider/model/credential |
| INF-003 | open | 统一 domain-owned Effect outcome、receipt、idempotency 和 reconciliation 语义 | D-015 contract | invoke 明确产生 `succeeded`、`failed` 或 `uncertain`；仅 owner 可用同 key 追加 `reconciled`；replay 不执行 Effect；不新建全局 EffectCoordinator | `cargo test --locked -p nomifun-agent-session --lib effect -- --test-threads=1` | 否 |
| INF-004 | open | 为后续业务表提供 Fresh-v4 版本化 schema evolution，而不是修改已发布 baseline | 现有 canonical Fresh-v4 root | 从当前 user_version 可重启升级；domain resource/outbox migration 有 digest、fault test 和双启动证据；旧 dev root 不被破坏 | `cargo test --locked -p nomifun-v4-root -- --test-threads=1` | 否 |
| INF-005 | open | 完成同一 `AgentPlatform` 实例的 canonical Session ServiceKey 消费者迁移；provider 与 Wave 5 exact manifest 已在 `3f835174` 落盘 | Kernel typed service registry | AgentExecution、Schedule、Requirement、IDMM、Channel、Remote 等消费者取得同一 session authority；不自行构造第二个 store/platform；production host 双启动通过 | `cargo test --locked -p nomifun-agent-platform session_services --lib` | 否 |
| INF-006 | open | 扩展 Preset resource editor、Preview 和 catalog 到剩余 resource kinds | 各 Wave resource schema | 用户无需手填内部 ID/path；owner、cardinality、operation、platform availability 在 Save/Test 前 fail-closed；已完成 Knowledge UI 不回退 | `bun test --cwd ui src/renderer/pages/agentSettings/model.test.ts` | 最终交互验收需要用户执行一次 |

## Wave 1：Research、Knowledge、Memory、Skill

缺失 action owner 精确为 6：`web.search`、`knowledge.write`、
`knowledge.autogen`、`knowledge.embedding`、`knowledge.rerank`、`skill.invoke`。

| ID | 状态 | 范围 / 目标 | 依赖 | 完成定义 | 最小验证命令 | 用户 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- |
| W1-001 | blocked | 实现真实 `web.search` owner | INF-001、按选型可能依赖 INF-002/INF-003 | query/limit 输入、规范化 hits、provider/config digest、ExternalTransmit receipt 与 SSRF/egress policy 均明确；无 fake search | `cargo test --locked -p nomifun-app wave1_web_search --lib -- --test-threads=1` | 需要搜索 provider/合同选择；live 验证需要凭据 |
| W1-002 | open | 闭合现有 `web.fetch` 的 ExternalTransmit result replay | INF-003 | 同 idempotency key + request digest 返回持久 receipt/result；冲突 key 拒绝；未知结果不自动重发；重启后可 replay | `cargo test --locked -p nomifun-app wave1_web_fetch --lib -- --test-threads=1` | 否 |
| W1-003 | open | 将 `knowledge.write` 输入拆成明确的 Create 与 Append command | 无 | `CreatePath` 遇已存在文件返回 conflict；`AppendHandle` 只接受匹配 KnowledgeBase 的 opaque handle；删除可变 display-name `base` 寻址和隐式 upsert | `cargo test --locked -p nomifun-agent-domain-wave1 knowledge_write --lib` | 否 |
| W1-004 | open | 实现 root-anchored no-follow Knowledge mutation primitive | W1-003 | 逐目录 handle 创建/打开；create-new 不覆盖；append 不逃逸；拒绝 symlink/junction/reparse；mutation 开始后不使用 timeout-and-drop | `cargo test --locked -p nomifun-knowledge anchored_write --lib -- --test-threads=1` | macOS/Linux fault evidence 后续需要真实主机 |
| W1-005 | blocked | 解决 `knowledge.write` TOCTOU/CAS、publication commit point 和 durable reconcile blocker | INF-003、W1-004 | 不再宣称“读取比较后 rename”是跨外部编辑器原子 CAS；采用可证明的单一写 authority 或 immutable version；返回 NotApplied/AppliedNotDurable/OutcomeUnknown 等内部结果并持久化 `started -> terminal/uncertain -> reconciled`；journal 不因固定 128 条永久失能 | `cargo test --locked -p nomifun-knowledge bound_knowledge_write --lib -- --test-threads=1` | 需要产品接受最终并发写语义；不需要明文凭据 |
| W1-006 | blocked | 实现 `knowledge.autogen` owner | INF-002、W1-005 | 从 bound KB 安全采样，经 exact one-shot route 生成 description/README；写入走 canonical Knowledge write；保存 model/config provenance 和 write receipt | `cargo test --locked -p nomifun-knowledge autogen --lib -- --test-threads=1` | live 验证需要 Chat provider |
| W1-007 | blocked | 冻结并实现 `knowledge.embedding` 的准确语义 | INF-001、INF-002 | 明确是返回 raw vector 还是对 bound KB 做 semantic retrieval；DTO/outcome、维度校验、route provenance、resource ownership 与缓存失效均固定 | `cargo test --locked -p nomifun-knowledge embedding --lib -- --test-threads=1` | 需要语义确认；live 验证需要 embedding provider/model |
| W1-008 | blocked | 实现 `knowledge.rerank` owner | INF-001、INF-002、W1-007 | DTO 包含 query、documents/handles、top_n；拒绝重复/越界索引；outcome 返回稳定排序、score 和 route provenance | `cargo test --locked -p nomifun-knowledge rerank --lib -- --test-threads=1` | live 验证需要 rerank provider/model |
| W1-009 | open | 实现 `skill.invoke` 及 exact Skill artifact loading | canonical SkillDefinition / snapshot lock | bundled Skill inventory 物化为 exact version/body/resource digest；invoke 只加载 Snapshot 锁定 artifact；参数语义和 typed outcome 固定；不通过旧 CLI workspace link 冒充 invoke | `cargo test --locked -p nomifun-agent-domain-wave1 skill --lib -- --test-threads=1` | 需要确认首发 bundled Skill inventory |
| W1-010 | open | 核查并接线 Wave 1 的 11 个 non-action contribution | W1-006、W1-009 按适用依赖 | `citation.render`、attachments、Knowledge mount/source sync、Memory read/citation/scratch/recall、Skill catalog/describe/hooks 均有真实 Fresh-v4 contributor 或明确 exact-unavailable；不以 inventory metadata 代替功能 | `cargo test --locked -p nomifun-agent-domain-wave1 --lib` | 否 |

## Wave 2：Workspace、VCS、SSH、MCP、Browser、Computer

缺失 action owner 精确为 15：SSH 4 个、MCP 1 个、Connector 2 个、Browser 6 个、
Computer 2 个。`vcs.push` 已有 local/file remote production owner，但其网络 credential
authority 和 reconcile 子项仍在 `W2-001` 跟踪。

| ID | 状态 | 范围 / 目标 | 依赖 | 完成定义 | 最小验证命令 | 用户 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- |
| W2-001 | open | 补齐 `vcs.push` 网络 credential authority 与 explicit reconcile；local/file remote owner 已在 `8aade375` 落盘 | INF-003、workspace/VCS owner | exact remote/ref、credential authority、force policy 和 ExternalTransmit receipt 固定；未知结果不自动 push；不得默认 force；SSH/HTTPS live evidence 通过 | `cargo test --locked -p nomifun-app vcs_push --lib -- --test-threads=1` | live 验证需要可丢弃 network remote repo/credential |
| W2-002 | open | 实现 `ssh.fs.read/write`、`ssh.exec`、`ssh.sudo` 四个 owner | INF-003、`ssh_host` binding | owner/host-key/credential/operation grant 校验；bounded IO；cancel/timeout 后进程与 channel 可证明回收；sudo 无交互等待旁路 | `cargo test --locked -p nomifun-ssh --lib` | live 验证需要测试 SSH host 与凭据 |
| W2-003 | open | 实现 `mcp.tool_proxy` 并闭合 connect/resource/OAuth contribution | MCP server binding、INF-003 | exact server/config/tool schema 被 Snapshot 锁定；OAuth secret 不入 Snapshot/Event；disconnect/timeout/result replay typed fail-closed | `cargo test --locked -p nomifun-mcp --lib` | live 验证需要测试 MCP server/OAuth 配置 |
| W2-004 | open | 实现 `connector.data.read/write` owner | W2-003、INF-003 | connector 数据 contract 不复用任意 MCP JSON；write 有 domain receipt/idempotency；read 有 bounded result/provenance | `cargo test --locked -p nomifun-agent-domain-wave2 connector --lib` | 真实 connector 验证需要外部服务 |
| W2-005 | open | 实现六个 Browser action 及 identity/observe/site-memory contribution | browser resource、platform adapter、INF-003 | navigate/act/download/upload/evaluate/takeover 走一个 owner；下载上传受 workspace binding 约束；Windows/macOS/Linux Desktop availability exact；Headless exact-unavailable | `cargo test --locked -p nomifun-browser-platform --lib` | native UI/browser 验证需要对应 Desktop 主机 |
| W2-006 | open | 实现 `computer.input/launch` 及 observe/a11y contribution | computer resource、OS permission adapter、INF-003 | Windows/macOS 完整 owner；Linux Desktop 与 Headless 按 frozen matrix exact-unavailable；权限拒绝、取消和进程清理有 typed outcome | `cargo test --locked -p nomifun-agent-domain-wave2 computer --lib` | 需要真实 Windows/macOS 权限人工验证 |
| W2-007 | open | 闭合 Wave 2 其余 non-action contribution 与现有 `process.exec` 生命周期 | INF-006、native cells | `fs.watch`、workspace bind/artifacts、process session、terminal PTY 等不是 metadata-only；`process.exec` 在五格适用平台完成 timeout/cancel/descendant cleanup evidence | `cargo test --locked -p nomifun-app agent_wave2_host --lib -- --test-threads=1` | native lifecycle 需要真实目标主机 |

## Wave 3：Creation、Workshop、Office、MiniApp

本 Wave 的 19 个 capability 全部是 action-bearing，目前 0 个 production owner。

| ID | 状态 | 范围 / 目标 | 依赖 | 完成定义 | 最小验证命令 | 用户 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- |
| W3-001 | open | 完成 Wave 3 action contract 冻结；`d1acccf6` 已覆盖 19 个映射并冻结其中 15 个 | INF-003 | `workshop.director`、`office.document.edit`、`office.sheet.edit`、`office.slides.edit` 不再显式 blocked；19 个 action 均有 bounded input、资源引用、success/failure/uncertain 结果和 schema digest | `cargo test --locked -p nomifun-agent-domain-wave3 --lib` | 需要产品确认 Director 与 Office mutation 的最小生产语义 |
| W3-002 | open | 建立 Fresh-v4 Canvas、Asset、GenerationTask/Provider、TemplateRun、Office artifact、MiniApp resource/repository/UI binding | INF-004、INF-006 | schema、owner、operation、outbox、delete non-cascade 和 Preview 校验完整；旧业务表不直接注入 | `cargo test --locked -p nomifun-v4-root wave3 -- --test-threads=1` | 否 |
| W3-003 | open | 实现 5 个 Creation owner：text/image/image-edit/video/audio | W3-001、W3-002、INF-001、INF-002 | exact generation route、input asset、task state、artifact digest 和 result receipt 可恢复；provider unknown result 不猜测成功 | `cargo test --locked -p nomifun-creation --lib` | live 验证需要相应生成 provider/model |
| W3-004 | open | 实现 6 个 Workshop owner | W3-001、W3-002 | Canvas/Asset read-edit、template run、director 均只操作 bound resource；编辑可重放结果但不重复 Effect；并发冲突明确 | `cargo test --locked -p nomifun-workshop --lib` | 最终画布交互需人工验收 |
| W3-005 | open | 实现 4 个 Office owner | W3-001、W3-002 | preview/document/sheet/slides 使用 canonical artifacts；输入输出 bounded；保存、失败和冲突有 typed receipt | `cargo test --locked -p nomifun-office --lib` | 最终文档预览需人工验收 |
| W3-006 | open | 实现 4 个 MiniApp owner | W3-001、W3-002 | read/edit/publish/serve 使用 immutable publish snapshot；serve 不读取 working copy；publish 有 idempotent receipt 与 rollback-free provenance | `cargo test --locked -p nomifun-miniapp --lib` | 最终 publish/serve 需人工验收 |

## Wave 4：Channel、Companion、Customer Service、Robot、Notification

缺失 action owner 精确为 11：Channel 2、Companion 3、Customer Service 3、Robot 3。

| ID | 状态 | 范围 / 目标 | 依赖 | 完成定义 | 最小验证命令 | 用户 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- |
| W4-001 | open | 建立四域 v4 resource、domain event/outbox/inbox/idempotency persistence；typed Effect contract 已在 `765d1953` 落盘 | INF-003、INF-004、INF-005 | reliable 事实由 owning-domain transaction/outbox 保存；EventBus 只唤醒；uncertain 不重试；Session delete 不级联业务 receipt/outbox；11 个合同不回退 | `cargo test --locked -p nomifun-agent-domain-wave4 --lib` | 否 |
| W4-002 | open | 实现 Channel reply/send owner，并闭合 receive/pairing/group-policy contribution | W4-001、W5-006 | incoming message 映射 canonical AgentSession；reply/send 有 transport receipt、dedupe、disconnect recovery；配对确认仍归 Channel 产品策略 | `cargo test --locked -p nomifun-channel --lib channel -- --test-threads=1` | 需要至少一个真实 Channel 测试账号 |
| W4-003 | open | 实现 Companion summon/learn/evolve owner及 persona/roster contribution | W4-001、INF-005 | Companion 与 memory resource owner 一致；学习/进化 effect 可归因和 reconcile；不构造旧 Conversation/Nomi runtime | `cargo test --locked -p nomifun-companion --lib` | 最终 Companion 交互需人工验收 |
| W4-004 | open | 实现 Customer Service notes read/write/handoff 及 dialogue contribution | W4-001、INF-005 | customer binding、notes version、handoff receipt 和 AgentSession provenance 完整；旧 customer/conversation graph 不可达 | `cargo test --locked -p nomifun-customer-service --lib` | 最终 handoff 需要测试业务数据 |
| W4-005 | open | 实现 Robot display/motion/device-tools 及 link/audio/vision contribution | W4-001、INF-005 | device binding 与 owner 校验；每个物理 Effect 有 receipt/uncertain/reconcile；离线和 cancel fail-closed；不模拟硬件成功 | `cargo test --locked -p nomifun-robot --lib` | 真实闭合需要支持的 Robot/设备 |
| W4-006 | open | 闭合 notification.webhook/desktop 的 reliable delivery owner | W4-001 | webhook 使用 outbox、idempotency 和 result receipt；desktop notification 明确平台 availability；不依赖 EventBus 必达 | `cargo test --locked -p nomifun-agent-domain-wave4 notification --lib` | webhook live 验证需要测试 endpoint |

## Wave 5：AgentExecution、AutoWork、Schedule、Requirement、IDMM、Remote

缺失 action owner 精确为 10：AgentExecution 5、`schedule.store` 1、Requirement 4。
IDMM、AutoWork timer/trigger 与 Remote/Ingress 是 non-action contribution，另行闭合。

| ID | 状态 | 范围 / 目标 | 依赖 | 完成定义 | 最小验证命令 | 用户 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- |
| W5-001 | blocked | 实现 delegate/fork/plan/steer/observe 五个 AgentExecution owner | INF-005、INF-003 | 全部调用 canonical AgentSession command/query；fork 产生新 UUIDv7；不经 `ConversationService` 或 Nomi registry；process_session binding exact | `cargo test --locked -p nomifun-agent-execution --lib` | 否 |
| W5-002 | blocked | 实现 `schedule.store` 并闭合 timer/agent-trigger scheduler contribution | INF-005、INF-003、INF-004 | schedule 保存 exact Preset revision/Snapshot/resource binding；触发创建 canonical Session；重复触发有 durable idempotency；无旧 Cron Conversation path | `cargo test --locked -p nomifun-cron --lib` | 时间触发人工验收可由用户加速执行 |
| W5-003 | blocked | 实现 requirements read/write/status/claim owner | INF-005、INF-003、INF-004 | Requirement repository 与 AgentSession 分离；claim/status 并发语义明确；AutoWork 使用 typed command/receipt；不注入旧 RequirementService facade | `cargo test --locked -p nomifun-requirement --lib` | 需要一组可丢弃 Requirement 数据做人工验收 |
| W5-004 | blocked | 将 AutoWork runner 迁到 canonical Session/Schedule/Requirement ports | W5-001、W5-002、W5-003 | runner 不构造 ConversationService；每次计划/执行/终态可由 canonical SessionEvent + domain facts 恢复；cancel deadline 有界 | `cargo test --locked -p nomifun-requirement autowork --lib -- --test-threads=1` | 否 |
| W5-005 | blocked | 将 IDMM observe/intervene/fallback middleware 迁到 canonical Session ports | INF-005、INF-003 | middleware 只观察 canonical session/query；intervene 走 typed command；无 legacy conversation ID、runtime registry 或隐式 fallback authority | `cargo test --locked -p nomifun-idmm --lib` | 否 |
| W5-006 | blocked | 闭合 Remote REST/MCP 与 ingress.web/mobile/channel 的产品主链 | W5-001、SCN-002～SCN-008、TST-009 | `open/turn/observe/cancel` 只用 explicit AgentSession ID；binding update no-drift、delete、disconnect cursor/idempotency 和 token rotate/revoke 全矩阵通过；无 `/mcp-agent`/selector/最近会话旁路 | `cargo test --locked -p nomifun-app remote --lib -- --test-threads=1` | 真实 ready/turn/dispose 需要 Windows Sidecar；Channel ingress 需要测试账号 |

## 旧组合图物理拆除

以下工作不能用重命名、allowlist、deprecated facade 或 mock success 关闭。删除前必须先迁移
全部真实消费者，避免把可达旧图变成隐藏 fallback。

| ID | 状态 | 范围 / 目标 | 依赖 | 完成定义 | 最小验证命令 | 用户 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- |
| LEG-001 | blocked | 停止由 `AppServices` 手工组合旧业务与 Nomi runtime | Wave 1～5 owners、INF-002、INF-005 | production Fresh-v4 startup 不取得 legacy business service bag；`AppServices` 仅剩待 C9 删除的不可达兼容代码或被完全移除 | `cargo check --locked -p nomifun-app` | 否 |
| LEG-002 | blocked | 删除 `router/state.rs` 中多个 `ConversationService::new` 与 late wiring | W5-001～W5-005、W4-002～W4-005 | Route、Cron、Requirement、Companion、Channel 等都复用 canonical AgentPlatform ports；构造器 residual 为 0 | `cargo test --locked -p nomifun-app router::state --lib -- --test-threads=1` | 否 |
| LEG-003 | blocked | 移除宽 `GatewayDeps` compatibility capability host | Wave 1～5 owners | 每个 handler 只依赖 manifest-declared typed service/resource view；production composition 不再投影完整 service bag | `cargo test --locked -p nomifun-gateway --lib` | 否 |
| LEG-004 | blocked | 将 Gateway capability handlers/facades 切到 v4 package owner 并删除旧 registry surface | LEG-003、各 Wave owner | 旧 capability ID、DTO、surface、global registry dispatch 和 owner bypass residual/reachability 为 0 | `cargo test --locked -p nomifun-gateway --lib` | 否 |
| LEG-005 | blocked | 迁移 AgentExecution、Cron、Requirement、AutoWork 的全部真实消费者 | W5-001～W5-004 | UI、API、background loop、MCP/Channel trigger 均只调用 canonical v4 ports；typed delegator 底层不再包旧 Conversation/Nomi | `cargo check --locked -p nomifun-agent-execution -p nomifun-cron -p nomifun-requirement` | 否 |
| LEG-006 | blocked | 迁移 Companion、IDMM、Channel 的全部真实消费者 | W4-002、W4-003、W5-005、W5-006 | message loop、supervisor、companion thread 与 UI route 不再依赖 legacy Conversation facts；领域业务事实保留在 owner repo | `cargo check --locked -p nomifun-companion -p nomifun-idmm -p nomifun-channel` | 真实 Channel 人工验证需要用户账号 |
| LEG-007 | blocked | 删除 `legacy_conversation_port`、旧产品 routes/DTO/table mapping/config/feature branches | LEG-002、LEG-004～LEG-006 | v4 不发布 alias、compatibility view、dual read/write 或旧 Session identity；published old migrations 仅保留历史源码且不进入 v4 runner | `bun run gate:agent-v2 -- c7-domain-waves` | 否 |
| LEG-008 | blocked | 使旧 Nomi factory/runtime 与 migration adapter 对生产消费者不可达，并为 C9 物理删除准备 | LEG-001～LEG-007、SCN-011 | 普通产品 Session 只能选择 Codex-derived Runtime；Nomi 仅在设计允许的 migration-only 范围存在且无产品 route/fallback；C9 前 residual 清单精确 | `bun run gate:agent-v2 -- c8-win-pre` | 最终删除依赖 Sidecar 与五格 evidence |

## Sidecar 与 Native

普通 upstream `codex.exe`、app-server、重命名二进制、cross-compile、VM、容器、Rosetta 或
synthetic evidence 均不能关闭下列条目。

| ID | 状态 | 范围 / 目标 | 依赖 | 完成定义 | 最小验证命令 | 用户 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- |
| SCN-001 | external | 从冻结 upstream `dc2ccc68` 建立真实 NomiFun Codex fork 分支与实际 patch series | Codex fork repository 写权限 | remote/branch 存在；六个 patch 文件有真实内容与 SHA；构建严格使用 pinned commit，不再只有意图记录 | `cd ..\codex\codex-rs; git status --short; git rev-parse HEAD` | 需要独立 Codex fork 仓库与推送权限 |
| SCN-002 | open | 闭合 NomiFun Runtime 输入合同 | AgentSession/Snapshot/Tool contracts | `create/start_turn/steer/resume/fork` 携带实际 input、exact model route、instructions/context、Tool/active-set projection、bounded history 和 Bridge endpoint；不再只有 ID/digest | `cargo test --locked -p nomifun-agent-contracts runtime --lib` | 否 |
| SCN-003 | external | 实现专用 `nomifun-codex-runtime` binary 与稳定 RPC facade | SCN-001、SCN-002 | 仅接受 `app-server --listen stdio://`；先完成 `runtime/hello`；只暴露冻结的八个 RPC；未知方法、raw upstream notification 和交互请求 fail-closed | `cd ..\codex\codex-rs; just test -p codex-app-server` | 需要 Codex fork 源码 |
| SCN-004 | external | 实现 inherited credential reader 与 audience-bound Model Bridge bootstrap | SCN-003 | Unix fd / Windows duplicated HANDLE 单次限长读取、清零并关闭；payload 含 loopback endpoint 与 audience token；不读全局 Codex auth/config | `cargo test --locked -p nomifun-codex-runtime credential --lib` | 需要 Codex fork 源码和真实 Windows handle 验证 |
| SCN-005 | blocked | 接入真实 Responses Bridge 并删除 Host 并行文本模型路径 | SCN-002～SCN-004、canonical ChatModelBroker | Sidecar 的真实 Codex model step 只经 loopback Bridge；`runtime_chat_bridge.rs` 不再在 ACK 后另跑一次文本模型；route/credential authority 唯一 | `cargo test --locked -p nomifun-agent-platform runtime_chat_bridge --lib` | live 验证需要测试 provider/model/credential |
| SCN-006 | external | 实现 native action ACK-before-effect | SCN-003、INF-003 | `apply_patch`、file edit、exec、stdin、commit、push 在副作用前发送 `native_action/start`；ACK timeout/mismatch/disconnect 时副作用次数为 0 | `cd ..\codex\codex-rs; just test -p codex-core` | 需要 Codex fork 源码 |
| SCN-007 | external | 实现 RuntimeEvent mapper、严格序列和未 ACK resend | SCN-003 | upstream Turn/Item/Tool 输出映射为 canonical `RuntimeEventEnvelope`；producer sequence 严格；未 ACK 保留并重发；不泄漏 upstream notification | `cargo test --locked -p nomifun-codex-runtime event --lib` | 需要 Codex fork 源码 |
| SCN-008 | external | 实现幂等 `runtime/session/dispose` | SCN-003 | shutdown Thread、turn、terminal、PTY、MCP、Code Mode、subagent/browser 资源；重复调用返回同一 ACK；清理不完整不得返回 `disposed=true` | `cargo test --locked -p nomifun-codex-runtime dispose --lib` | 需要 Codex fork 源码/制品 |
| SCN-009 | external | 完成 Windows process-tree fault proof | SCN-006、SCN-008 | normal dispose、ACK timeout、Sidecar crash、Host crash、breakaway attempt 后 Job 成员为 0；仅父 PID 退出不算证明 | `bun scripts/validation/check-windows-x64-native.mjs --sidecar <path> --run-sidecar-rpc` | 需要 Windows native helper 与真实 Sidecar |
| SCN-010 | external | 生成真实 Codex release artifacts 与 provenance | SCN-001～SCN-009 | Codex `Cargo.lock`、patch set、binary、hello、LICENSE、NOTICE、SBOM 均使用真实内容 digest；移除 `fixture_digest` 和错误的 NomiFun lock digest | `cargo test --locked -p nomifun-codex-runtime release --lib` | 需要 Codex fork、构建制品和 license/SBOM 输入 |
| SCN-011 | external | 关闭 `windows_desktop_x64` native cell与真实主链 | Wave 1～5、SCN-001～SCN-010、TST-001/002 | clean final tuple 上 build/package/install/fresh/upgrade/offline/full Coding/lifecycle/fault/process 全通过，并完成真实 open→ready→turn→tool/effect→observe→cancel→dispose | `bun run gate:agent-v2 -- c8-win-pre` | 需要 Windows x64 Sidecar、hello metadata 和测试 provider/binding/token |
| SCN-012 | external | 关闭 `macos_desktop_arm64` native cell | SCN-010、SCN-011 后 HP-1 | 真 Apple Silicon、非 Rosetta；Universal arm64 leaf、Sidecar、package/install/full Coding/anchored FS/lifecycle/fault 全通过 | `bun run gate:agent-v2 -- c8-ma --evidence <repo-relative-evidence>` | 需要 Apple Silicon Mac、arm64 Sidecar、签名/打包环境 |
| SCN-013 | external | 关闭 `macos_desktop_x64` native cell | SCN-010、C8-MA/HP-2 | 真 x64 Mac；同一 Universal App 的 x64 leaf 和 x64 Sidecar 完整通过；不能由 Rosetta 代验 | `bun run gate:agent-v2 -- c8-mx --evidence <repo-relative-evidence>` | 需要 Intel Mac、x64 Sidecar、签名/打包环境 |
| SCN-014 | external | 关闭 `linux_desktop_x64` native cell | SCN-010、HP-2 | GNU Desktop Host + required Sidecar 完成 package/install/full Coding/Browser availability/lifecycle/fault/process；Computer exact-unavailable | `bun run gate:agent-v2 -- c8-ld --evidence <repo-relative-evidence>` | 需要真实 Linux Desktop x64 主机与 Sidecar |
| SCN-015 | external | 关闭 `linux_headless_x64` native cell | SCN-010、HP-2 | GNU Headless Host + required Sidecar 完成 install/full Coding/lifecycle/fault/process；Browser/Computer exact-unavailable | `bun run gate:agent-v2 -- c8-lh --evidence <repo-relative-evidence>` | 需要真实 Linux Headless x64 主机与 Sidecar |

最终 required native cell 为 `SCN-011`～`SCN-015` 五格：Windows x64、macOS arm64、
macOS x64、Linux Desktop x64、Linux Headless x64。当前正式 C8 final-cohort evidence
为 **0/5**；已有 macOS arm64 工程检查不是 `C8-MA` PASS。

## 测试与 Gate

| ID | 状态 | 范围 / 目标 | 依赖 | 完成定义 | 最小验证命令 | 用户 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- |
| TST-001 | open | 清理 repository-wide UI typecheck baseline | 无；按 changed surface 分批 | React/Arco/implicit-any baseline 为 0，`bun run check` 不再产生 `baseline_fail`；production build 保持通过 | `bun run typecheck` | 否 |
| TST-002 | open | 定位 `canonical_remote_rest_freezes_binding_and_auth_fence` 挂起 | 禁止直接重跑 workspace Gate | 使用现有 60 秒总 deadline 单独运行一次可得到 PASS 或确定失败点；compose/open/revoke/close 每段有诊断；不以 timeout 当 PASS | `cargo test --locked -p nomifun-app bootstrap::canonical_host::tests::canonical_remote_rest_freezes_binding_and_auth_fence --lib -- --exact --test-threads=1` | 如需要真实 Sidecar 才能复现，转交用户提供 SCN-011 输入 |
| TST-003 | pending-validation | 完成 macOS x64 与 Linux 的 anchored Knowledge 原生复验；macOS arm64 已在 `f46cc017` 记录通过 | `1a547f3a`、对应 native host | symlink/path replacement、directory handle、enumeration 和 limits 在 macOS arm64/x64、Linux x64 全通过；Windows 不能代签；最终候选变更后按 affected-cell 规则复验 | `cargo test --locked -p nomifun-knowledge service::anchored_fs::tests --lib -- --test-threads=1` | 仍需要 Intel Mac 与 Linux x64；Apple Silicon 定向 evidence 已有 |
| TST-004 | pending-validation | 完成 Agent Preset Knowledge binding UI 交互 evidence | `280841b3` | 真实 KnowledgeBase 可选、Draft/Preview 正确、无 UUID/path/owner error；Desktop 不崩溃；可用人工结果替代当前损坏的 Playwright harness | `bun run dev` | 需要用户人工操作；macOS 当前缺 Accessibility/Screen Recording TCC 权限 |
| TST-005 | blocked | 七个官方模板和全部 required domains 的代表性 all-scene E2E | Wave 1～5 owners、SCN-011 | chat/assistant/coding/companion/robot/customer-service/creative-studio 均通过真实 owner、真实 resource/provider failure 和 Session lifecycle；无 metadata-only success | `bun run gate:agent-v2 -- c8-win-pre` | 需要测试 provider、资源和设备/Channel 按场景提供 |
| TST-006 | blocked | 完成 Effect、receipt、uncertain/reconcile、replay-no-effect fault matrix | INF-003、W1/W2/W3/W4 effect owners | crash 点、unknown result、restart、same-key replay、conflicting key、EventBus drop 均有确定结果；外部 Effect 执行次数可证明 | `cargo test --locked -p nomifun-agent-session --lib effect -- --test-threads=1` | 某些 live Effect 需要外部 sandbox 资源 |
| TST-007 | open | 复核 D-024 unified delete/tombstone/late-callback matrix | canonical Session delete 已存在 | fence→cancel/dispose→zero handles→private purge→四字段 tombstone；deleted ID 全操作稳定 `SESSION_DELETED`；domain receipt/outbox non-cascade | `cargo test --locked -p nomifun-agent-session --lib delete -- --test-threads=1` | 否 |
| TST-008 | open | 复核 D-025 compatibility/resume/fork matrix | canonical Snapshot/runtime contracts | exact compatible executor 可继续原 Session；不兼容只读并显式 fork；checkpoint mismatch discard；Tool/Effect replay 为 0 | `cargo test --locked -p nomifun-agent-session --lib compatibility -- --test-threads=1` | 最终 Coding continuation 需要 Sidecar |
| TST-009 | blocked | 完成 D-026 Remote request-admission fence 全矩阵 | W5-006、SCN-011 | REST/MCP 四操作在 rotate/revoke commit 前后顺序正确；旧 token 后续 admission 全拒绝；既有 Session 不被改写 | `cargo test --locked -p nomifun-app remote_auth --lib -- --test-threads=1` | 真实 ready/turn 路径需要 Sidecar 与 installation token |
| TST-010 | blocked | 在最终 clean Windows tuple 运行一次完整 C8-WIN-PRE | Wave 1～5、LEG、SCN-011、TST-001/002 | required checks 全 pass；owner blockers 空；workspace test 完成；Windows package/native evidence 有效；不重复相同 tuple | `bun run gate:agent-v2 -- c8-win-pre` | 需要用户确保机器空闲并提供 Sidecar/provider 资源 |
| TST-011 | blocked | 使 C8 owner coverage、global residual/reachability 与 outstanding ledger 达到准入条件 | 所有 owner、LEG、REL-005 | 五个 fallback/unconfigured marker 不再 production reachable；C8 scope blocking/unclassified/canonical owner residual 为 0；C9-deferred 精确归档 | `bun run gate:agent-v2 -- c8-win-pre` | 否 |
| TST-012 | open | 维护测试障碍台账并停止盲目重试 | 无 | Windows `fmt --all` error 206、Channel 150 秒 timeout、历史 ENOBUFS/access violation、Playwright ESM、既有 clippy warnings 均被修复或明确证明不属于对应 Gate；保留首个完整日志和手动替代步骤 | `git diff --check` | 用户可执行耗时人工验证；不要求反复机器重试 |

## C9 与 Release

| ID | 状态 | 范围 / 目标 | 依赖 | 完成定义 | 最小验证命令 | 用户 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- |
| REL-001 | blocked | 在代码稳定后重新冻结 canonical cohort tuple、digests、manifest 和 clean remote SHA | TST-010 前的全部代码完成 | source SHA、decision contract、platform manifest、runtime release、Cargo.lock 和生成 ledger 一致；worktree clean；origin branch 与本地 HEAD exact-match | `cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check` | push 时需要网络/GitHub 可用 |
| REL-002 | blocked | 完成 C8-WIN-PRE 后执行 HP-1 | REL-001、TST-010 | Windows full Gate pass；普通 push；`git ls-remote` 验证 SHA；只在此时正式移交 macOS arm64 | `git ls-remote origin refs/heads/rf/agent-capability-platform-v2` | 需要用户切换到真实 Apple Silicon Mac |
| REL-003 | external | 完成 C8-MA 并执行 HP-2 | REL-002、SCN-012 | 整个 candidate 在 macOS arm64 full native Gate 通过；fix 批量记录 affected cells；普通 push/remote SHA 验证后再启动其他三格 | `bun run gate:agent-v2 -- c8-ma --evidence <repo-relative-evidence>` | 需要 Apple Silicon Mac 与完整原生输入 |
| REL-004 | external | 完成 C8-MX/C8-LD/C8-LH、必要 C8-RECHECK-n，并聚合 C8-MERGE | REL-003、SCN-013～SCN-015、REL-005 | 五格同一 tuple；pending/fail/stale=0；affected full Gate、unaffected native scoped attestation 完整；global residual zero | `bun run gate:agent-v2 -- c8-merge` | 需要 Intel Mac、Linux Desktop、Linux Headless 和整轮回传 |
| REL-005 | blocked | 执行 D-027 final stop-admission、finite drain、dispose/kill descendants、exact-zero | 五格 candidate evidence、SCN-008、LEG-008 | fence 前 accepted operation 只到既有 finite deadline；之后 Runtime/Effect/task/resource/process/private-write outstanding 精确为 0；不 retry/replay uncertain | `bun run gate:agent-v2 -- c8-merge` | 需要 validation coordinator 安排最终 drain 窗口 |
| REL-006 | blocked | 创建 `C9-HARD-DELETE-MANIFEST.json` 并物理删除剩余 Nomi/migration coordinator/530 类 deferred residual | REL-004、REL-005 | C9 Gate 取得 same-source C8-MERGE、clean source、deletion manifest 和 D-027 zero proof；Nomi wiring/factory/runtime/private session/checkpoint/product fallback residual 为 0 | `bun run gate:agent-v2 -- c9-hard-delete` | 不需要旧数据迁移；删除窗口需用户确认候选已冻结 |
| REL-007 | blocked | 构建并验证五格同一 signed Nomi-free RC，处理 C10-RECHECK-n 与 C10-MERGE | REL-006 | C10-WIN/MA/MX/LD/LH 对同一 source/artifact/digest 全 pass；pending/fail/stale=0；C8 pre-delete evidence 不冒充 C10 | 按 release manifest 执行 `C10-WIN/MA/MX/LD/LH` Gate | 需要五个真实 native 环境与签名凭据 |
| REL-008 | blocked | C11 same-signed-digest Stable promotion | REL-007 | Stable 直接提升 C10-MERGE 已验证制品；source/binary/schema/UI/runtime/event/release digests 不变；无重新构建和 Nomi rollback | 执行 C11 release digest verification | 需要用户/发布负责人执行最终 promotion |

## 推荐执行顺序

1. 先关闭 `INF-001`、`INF-003`、`INF-004`、`INF-005`，同时独立推进
   `SCN-001～SCN-010` 的 Sidecar 源码/协议流。
2. 并行实现 Wave 2、Wave 3 resource/DTO、Wave 4 resource/outbox；每个领域先停在
   自己的 crate，不争抢中央 composition 文件。
3. 由单一 integration owner 接入 Wave 1～5 production hosts，再完成 Wave 5
   canonical Session consumer migration。
4. 关闭 `LEG-001～LEG-008`，此时再运行定向 residual/reachability 检查。
5. 修复 UI typecheck 与 Remote hang，只在最终 clean tuple 上运行一次 C8-WIN-PRE。
6. 严格按 `HP-1 -> C8-MA -> HP-2 -> C8-MX/LD/LH -> C8-RECHECK-n ->
   D-027/C8-MERGE -> C9 -> C10 five-cell -> C11` 推进。

在 `REL-006` 完成前不得宣称 Nomi 已删除；在 `REL-004` 完成前不得宣称 C8 或 HP-1
已完成；在 `REL-008` 完成前不得宣称发布完成。
