# Agent Capability Platform v2 一期精简闭合 TODO

> 盘点日期：2026-09-03
>
> 基线分支：`rf/agent-capability-platform-v2`
>
> 权威来源：`05-system-capability-replacement-foundation.zh.md`
>
> 当前结论：S0-S2 的止损、P0 与基础收缩已按定向 evidence 收口；MCP canonical owner
> 已在 S3 收口，Agent Settings 产品表面已在 S4 收口，其余 S3-S5 尚未完成。
> Sidecar live binary/credential、Windows 完整候选、C8、C9、Stable 或 Browser/Computer
> 可替换主链仍不得宣称完成。

本文是 05 发布后的唯一一期执行台账。旧版 84 个 `INF/W/LEG/SCN/TST/REL`
ID 从现在起只作为历史审计索引，不再是一期必须逐项关闭的阻断清单，也不得继续用
“81 个 action-bearing Capability 是否全部有 owner”、旧 residual 数量或五平台笛卡尔积
衡量一期完成度。

一期执行台账追踪 05 要求的 S0-S5：先停止错误扩张和审计普通 revert，再关闭三个 P0、
单 Compiler、小 Snapshot、三类 Effect、Sidecar upstream spike，随后完成
Browser/Computer Role seam、真实核心 owner、四条 UI 用户流程、Windows、macOS arm64、
Linux Desktop 和一次性 C9 clean cut。05 中的 `S6 Stable` 只是 S5 完成后的同制品发布提升
动作，不新增一组开发任务。

## 状态与执行规则

| 状态 | 含义 |
| --- | --- |
| `open` | 当前 owner 可以直接实施，不需要等待其他 TODO |
| `blocked` | 依赖项或当前必需的 live binary/credential 尚未具备；不得通过兼容层、mock 或 synthetic PASS 绕过 |
| `external` | 只有对应真实原生平台或签名环境才能产生发布证据；不是跨机开发任务 |
| `pending-validation` | 实施内容已形成，但尚待审查、提交或指定验证 |
| `closed` | 完成定义已满足，并有提交或可复查的最小 evidence |

执行约束：

1. 05 与本文冲突时以 05 为准；经修订的 01-04 与 `DECISIONS` 保留设计依据，但不记录实时
   状态。旧 `IMPLEMENTATION-STATUS`、旧 GLOBAL TODO、旧 Prompt 和 handoff 仅作 Git
   历史审计，不是当前执行材料。
2. 不使用 reset、force-push 或历史重写；revert 必须使用普通提交，并先检查真实消费者。
3. 每个任务只实现一个实际闭环；需要第二份事实、新 coordinator、新全局 digest 或新状态机
   时先停止并重新核对 05。
4. 非首批 Capability 可以保持明确 unavailable，但不得返回 metadata-only success，也不得
   阻塞核心 Stable。
5. API key、token、私钥、主机地址和签名 secret 不进入源码、文档、fixture、日志、命令行
   参数或报告。
6. 测试遇到环境或 harness 障碍时记录首个完整失败、停止盲目重试，并提供人工替代步骤。
7. 二期 `06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md` 不属于本台账，
   不得在一期实现、提交或引用为已冻结合同。

## 历史进度保留

以下是真实已完成或可复用的功能，不因止损而删除，但也不自动关闭后续集成任务：

| Commit | 保留内容 | 后续处置 |
| --- | --- | --- |
| `099893cc`、`56e70fd1d` | Fresh-v4 storage generation 与前端 bootstrap 启动修复 | 保留，继续作为 `bun run dev` 基线 |
| `745fabfa` | binding-backed `knowledge.search/read` owner | 保留真实 owner |
| `280841b3` | Agent Preset KnowledgeBase picker | 保留并纳入 S4 用户流程 |
| `5d691824` | anchored Knowledge 文件访问 | 保留基本 containment；不继续扩大极端本机攻击证明 |
| `3f835174`、`c6503a23` | canonical AgentSession command/query ServiceKey 及 core service package host 测试适配 | 保留单一 Session authority |
| `8aade375` | 真实 local/file `vcs.push` owner | 保留 owner；按三类 Effect forward 简化 |
| `b58a0f92` | fork cursor 修复 | 保留产品语义 |
| `dd07b937` | Remote public route 改用 Session command/query ports | 保留；auth fence 已由 `SL-S2-03` 关闭，真实 Runtime 产品链仍待 `SL-S3-11/12` |
| `efbcb598`、`23e039ff`、`1a547f3a`、`f46cc017` | Knowledge 的 Windows/macOS 工程验证 | 保留为工程记录，不冒充最终候选 native PASS |

已按止损结论处理的历史实现：

- `d1acccf6` Wave 3 批量 typed contract：已由 `2ad8ca12` 普通 revert。
- `765d1953` Wave 4 通用 Effect/receipt contract：已由 `8f4ba1d9` 普通 revert。
- 旧 C8/C10 cohort、handoff、fixture digest、五格 evidence 和 residual 分类实现：
  只保留修复 P0、三平台 RC 和发布追溯真正需要的最小部分；旧交接材料不再作为当前任务
  入口。

## 已完成基础切片与定向 evidence

下列项目已经在当前工作树中实现，并有可复查的定向 evidence。这里的 `closed` 只表示
对应基础切片满足完成定义，不表示 Sidecar live、Windows 候选、C8 或原生平台发布
已经通过：

| 项目 | evidence | 边界 |
| --- | --- | --- |
| `SL-S2-05` Session Projection | `cargo test --locked -p nomifun-agent-session --lib` | Projection 不复制完整 `events[]`；正常完成保存最终 assistant message，中断只保留 bounded partial |
| `SL-S2-06` 三类 Effect | `cargo test --locked -p nomifun-agent-session --lib` | 只保留 `read_only`、`managed_effect`、`external_uncertain_effect`；外部 unknown 不自动 retry |
| `SL-S2-07` canonical Compiler | `cargo test --locked -p nomifun-agent-control-plane --lib`；`cargo test --locked -p nomifun-agent-kernel --lib` | Preview/Save/Test 共用 Kernel canonical Compiler |
| `SL-S2-08` small Snapshot / Fresh-v4 projection | `cargo test --locked -p nomifun-v4-root -- --test-threads=1`；`cargo test --locked -p nomifun-agent-kernel --lib` | Snapshot 只锁实际执行闭包；无读者投影已删除 |
| `SL-S2-09` PluginRegistration | `cargo test --locked -p nomifun-agent-kernel materialize --lib` | metadata 从 Manifest 与真实 exports 派生，保留 typed dependency 与 cleanup |
| `SL-S2-10` official app-server spike | `a7ac1d124`；`bun scripts/validation/codex-app-server-spike.mjs --self-test`；`bun test scripts/validation/codex-app-server-spike.test.mjs` | 已确认 pinned upstream 协议；不等同于 exact binary 或 live model 验证 |
| `SL-S3-09` SSH owner primitive | `77bd45279`；`cargo check --locked -p nomi-ssh -p nomifun-ssh`；`cargo test --locked -p nomifun-ssh --lib` | 已实现有界输入、超时、取消回收和 no-retry；live sshd/sudo 未运行时不构造 PASS |
| `SL-S3-08` MCP owner/source | `cargo test --locked -p nomifun-mcp --lib`；`cargo test --locked -p nomifun-app --lib router::agent_wave2_mcp::tests -- --test-threads=1`；`cargo test --locked -p nomifun-app --lib router::agent_wave2_host::tests -- --test-threads=1` | v4 source、exact lock、Streamable HTTP owner、typed failure、no-redirect 和 bounded cleanup 已通过；OAuth/stdio 不在本项隐式扩张 |

## 汇总

| 阶段 | closed | open | blocked | external | pending-validation | 合计 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| S0 止损发布 | 2 | 0 | 0 | 0 | 0 | 2 |
| S1 Revert/keep 审计 | 3 | 0 | 0 | 0 | 0 | 3 |
| S2 P0 与基础收缩 | 10 | 0 | 0 | 0 | 0 | 10 |
| S3 Role seam 与核心 owner | 8 | 1 | 3 | 0 | 0 | 12 |
| S4 产品 UI | 1 | 0 | 1 | 0 | 0 | 2 |
| S5 三平台、C9 与 RC | 0 | 0 | 3 | 2 | 0 | 5 |
| **总计** | **24** | **1** | **7** | **2** | **0** | **34** |

旧台账 84 项现已收敛为 34 项。任务数量不是质量指标；只有完成定义和最小验证满足后
才能修改状态。

## 当前剩余 TODO 快照

当前还剩 10 项未关闭：

| 分类 | 数量 | TODO |
| --- | ---: | --- |
| 主机当前实施 | 1 | `SL-S3-07` |
| 依赖阻塞 | 7 | `SL-S3-10`～`SL-S3-12`、`SL-S4-02`、`SL-S5-01`、`SL-S5-04`、`SL-S5-05` |
| 外部原生环境 | 2 | `SL-S5-02` macOS arm64、`SL-S5-03` Linux Desktop x64 |

主机关键路径已完成 `SL-S3-01 -> SL-S3-02 -> SL-S3-03`，Browser/Computer
first-party dogfood 和具体实现旁路清理也已完成；当前主线是 `SL-S3-07`，再进入
automation/Remote/Sidecar 和候选验证。
上述主机项全部由当前主机执行。主机可以按互斥写集启用多个本机 lane；中央合同、
组合根、Gate、锁文件和 GLOBAL TODO 由集成 Owner 串行合流。外部 macOS/Linux 只验证
冻结候选，不领取开发任务，也不维护 Prompt、交接包、远端 SHA 或跨机 attestation。

## S0：止损发布

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S0-01` | closed | 主机 | 发布 05 并停止旧 84 项驱动的扩张 | 无 | 05 独立提交，明确覆盖旧 Gate、Effect、平台矩阵和 TODO 口径 | `git show --stat d6de5170` | 无 |
| `SL-S0-02` | closed | 主机 | 用本文替换旧 84 项阻断台账 | `SL-S0-01` | 只保留 S0-S5 stable ID、状态、owner、依赖、完成定义、最小测试和人工输入；统计自洽 | `df4bdf56`; `git diff --check -- docs/specs/2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md` | 无 |

## S1：Revert/keep 审计

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S1-01` | closed | 主机 | 审计并处理 `d1acccf6` Wave 3 批量合同 | `SL-S0-01` | 无真实 owner/repository/入口的批量合同已普通 revert；未来只按真实场景重新加入最小 DTO，不保留 alias | `git show --stat 2ad8ca12` | 无 |
| `SL-S1-02` | closed | 主机 | 审计并普通 revert `765d1953` Wave 4 通用 Effect 合同 | `SL-S0-01` | 过度 receipt/reconcile 状态机已从主线撤销，未回滚真实用户功能 | `git show --stat 8f4ba1d9` | 无 |
| `SL-S1-03` | closed | 主机 | 完成保留提交的 forward-simplify 清单 | `SL-S1-01` | 对 Session ServiceKey、Knowledge、VCS、Remote、lifecycle、Gate 分别标记 keep/delete/simplify；没有“因已有代码而继续兼容”的项目 | 下方 forward-simplify 决策表；`git diff --check` | 无 |

### Forward-simplify 决策

| 范围 | 决策 | 后续边界 |
| --- | --- | --- |
| AgentSession command/query ServiceKey | keep | 继续作为单一 Session authority，不再新增第二套 Remote/automation Session API |
| Knowledge search/read 与 anchored 文件访问 | keep + simplify | 保留真实 owner 和基本 containment；停止扩张极端本机 TOCTOU 证明 |
| VCS status/diff/stage/commit/push | keep + simplify | 保留真实 owner；外部 push 只保留 idempotency 与 unknown no-retry |
| Remote open/turn/observe/cancel | keep + simplify | 保留显式 Session 主链；删除跨 Response Body auth permit 和旧 selector 旁路 |
| Session delete/dispose lifecycle | simplify | 由 `SL-S2-04` 删除调用者伪造的 zero proof，保留真实 dispose report 与幂等 tombstone |
| Gate、manifest 与 native evidence | delete + simplify | 删除 source SHA 自引用、fixture release digest、五格首发阻断；只保留三平台 RC 所需的 source input、真实 release lock/result |

## S2：P0 与基础收缩

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S2-01` | closed | 主机 | 删除 candidate source SHA 自引用 | `SL-S1-03` | pre-run input 不写其自身 commit SHA；Gate 运行时读取 clean HEAD；post-run result 记录 source commit 与 Artifact digest | `cargo test --locked -p nomifun-agent-contracts --lib`; `cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check`; `bun run gate:agent-v2 -- --self-test` | 无 |
| `SL-S2-02` | closed | 主机 | 物理分离 schema fixture 与真实 release lock/result | `SL-S2-01` | fixture 明示 `fixture_only=true` 并使用假 digest；`release-lock.json` 只记录真实制品 digest；`platform-result.json` 只记录 source、target、suite、结果和日志引用；Runtime、Gate 与 macOS build 不再把 fixture digest 当发布输入 | `cargo test --locked -p nomifun-codex-runtime --lib`; `bun test scripts/release/release-lock.test.mjs scripts/validation/check-macos-arm64-native.test.mjs`; `bun run gate:agent-v2 -- c7-domain-waves` | 首个真实 lock/result 随 S5 候选打包生成，不阻塞本项实现关闭 |
| `SL-S2-03` | closed | 主机 | 修复 Remote token rotate/revoke Response Body 卡死 | 无 | validator generation/hash 是认证线性化点；请求只持有短生命周期同步状态锁，不跨 Response Body 持有 auth permit；mint/revoke 仅由 mutation gate 串行；旧 token 后续请求立即失败 | `cargo test --locked -p nomifun-auth remote_admission --lib`; `cargo test --locked -p nomifun-public --lib`; `cargo test --locked -p nomifun-app bootstrap::canonical_host::tests::canonical_remote_rest_freezes_binding_and_auth_fence --lib -- --exact --test-threads=1` | 无 |
| `SL-S2-04` | closed | 主机 | 简化 D-024 delete/dispose | `SL-S1-03` | 已删除调用者填写的 `ZeroOutstandingProof`；平台删除路径校验真实 `RuntimeDisposeReport` 身份；启动时发现 `deleting` 会幂等清理 Session 自有内容并完成 tombstone | `cargo test --locked -p nomifun-agent-session delete --lib -- --test-threads=1`; `cargo test --locked -p nomifun-agent-platform --test chat_minimal -- --test-threads=1`; `cargo check --locked -p nomifun-app` | 无 |
| `SL-S2-05` | closed | 主机 | 收缩 SessionEvent 与 Projection | `SL-S2-04` | Event Log 保留唯一语义事实；Projection 不复制完整 `events[]`；正常完成只持久化最终 assistant message，中断最多一份 bounded partial | `cargo test --locked -p nomifun-agent-session --lib` | 无 |
| `SL-S2-06` | closed | 主机 | 把 Effect 生命周期收敛为三种策略 | `SL-S1-01`、`SL-S1-02` | 仅保留 `read_only`、`managed_effect`、`external_uncertain_effect`；本地操作使用事务/CAS/原子文件；外部 unknown 不自动 retry；删除 Wave 级通用 journal/coordinator | `cargo test --locked -p nomifun-agent-session --lib` | live 外部 Effect 未在无授权环境中冒充通过 |
| `SL-S2-07` | closed | 主机 | 合并为一个 canonical Compiler | `SL-S1-03` | Preview/Save/Test 共用同一纯函数 Compiler；Session Open 读取已保存 Snapshot，只做当前执行兼容检查；删除第二份 closure/digest 算法 | `cargo test --locked -p nomifun-agent-control-plane --lib`; `cargo test --locked -p nomifun-agent-kernel --lib` | 无 |
| `SL-S2-08` | closed | 主机 | 缩小 Snapshot、CapabilitySelection 和 Fresh-v4 投影 | `SL-S2-07` | Snapshot 只锁实际 Capability/Provider/Tool/Model/resource/runtime 闭包；删除未执行 selection 字段和只写不读的重复投影；fresh-v4 fixture 可双启动 | `cargo test --locked -p nomifun-v4-root -- --test-threads=1`; `cargo test --locked -p nomifun-agent-kernel --lib` | 无 |
| `SL-S2-09` | closed | 主机 | 简化 PluginRegistration | `SL-S2-07` | Manifest 是声明事实源；registration metadata 从真实 handler/service exports 派生；保留 namespace、schema、typed dependency、duplicate/cycle 和 cleanup | `cargo test --locked -p nomifun-agent-kernel materialize --lib` | 无 |
| `SL-S2-10` | closed | 主机 | 完成 Codex official app-server upstream spike 并冻结最小 Sidecar 协议 | 无 | pinned source 已确认 initialize/thread/turn/interrupt/event、Host-managed Tool 和关闭语义；不预设历史自定义 RPC；spike 不代替 live binary/credential 验证 | `a7ac1d124`; `bun scripts/validation/codex-app-server-spike.mjs --self-test`; `bun test scripts/validation/codex-app-server-spike.test.mjs` | exact pinned binary 和 live credential 由 `SL-S3-12` 继续处理 |

## S3：Role seam 与核心 owner

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S3-01` | closed | 主机 | 冻结 Browser/Computer versioned Role 合同 | `SL-S2-09` | `ExecutionRoleId`、Role Contract、source-neutral Provider contribution、required/optional member、typed Context/Resource exports 只有一套 canonical Rust/schema 定义 | `cargo test --locked -p nomifun-agent-contracts role --lib`; `cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check`; `cargo test --locked -p nomifun-agent-kernel --lib` | 无 |
| `SL-S3-02` | closed | 主机 | 实现 installation binding、Revision override、Resolver 和 Snapshot exact lock | `SL-S2-07`、`SL-S2-08`、`SL-S3-01` | override 优先、缺省继承 installation default；精确 Provider/contract/contribution/resource 进入 Snapshot digest；Operation admission 可携带独立 exact lock；缺失明确失败且不 fallback | `cargo test --locked -p nomifun-agent-control-plane --lib`; `cargo test --locked -p nomifun-agent-kernel --lib`（含 alternate/provider/registry/resource drift） | 无 |
| `SL-S3-03` | closed | 主机 | 实现单一 RoleDispatcher 与 Tool/Context/Resource runtime seam | `SL-S3-02` | Kernel 第一次路由直接选 frozen Provider Mount；Agent 与 non-Agent Tool/Context/Resource 共用 exact resolver；使用 Provider config/state/service/resource；不 façade 二次调用、不重选、不 retry/fallback | `cargo test --locked -p nomifun-agent-kernel --lib`（18/18）; `cargo check --locked -p nomifun-app --features browser-use,computer-use` | 无 |
| `SL-S3-04` | closed | 主机 | 第一方 Browser dogfood 同一 Role 主链 | `SL-S3-03` | observe/navigate/act 和 hidden `browser.render_content` 经同一 Provider lock；保留 owner/lane/close/process cleanup；Provider 平台约束不写死在 façade | `cargo test --locked -p nomifun-app --features browser-use --lib browser_role_owner_runs_the_canonical_observe_navigate_act_render_chain -- --ignored --test-threads=1`；Wave2 owner/lifecycle tests；alternate Provider parity test | 无；本机 data URL 作为可访问测试页 |
| `SL-S3-05` | closed | 主机 | 第一方 Computer/A11y dogfood 同一 Role 主链 | `SL-S3-03` | observe/input 基线和可选 launch/a11y 经 exact Provider；按 target resource 串行；observation generation 过期 typed fail；无具体 Registry 旁路 | `cargo test --locked -p nomifun-app --features computer-use --lib computer_role_owner_runs_the_canonical_observe_input_chain -- --ignored --test-threads=1`；Computer serialization/generation/platform-unavailable tests；`cargo test --locked -p nomi-computer --lib -- --ignored --test-threads=1` | 本机 Windows Desktop/UI Automation 权限已通过；macOS 权限仍由外部主机验证 |
| `SL-S3-06` | closed | 主机 | 删除 Browser/Computer production concrete bypass | `SL-S3-04`、`SL-S3-05` | Wave 2 first-party Role owner 可用；Knowledge rendered source 只经 typed `browser.render_content`；Gateway Browser/Computer registry、capability module 和 standalone `mcp-computer-stdio` 已物理删除；Nomi-only legacy allowlist 不增长并等待 C9 | `cargo test --locked -p nomifun-knowledge --lib`（315）；`cargo test --locked -p nomifun-gateway --lib`（122）；`cargo test --locked -p nomifun-gateway --test production_bypass_audit`；`bun run check:browser-platform-boundary` | 无；旧 Gateway Browser/Computer 工具不再作为兼容入口提供 |
| `SL-S3-07` | open | 主机 lane | 收口真实核心本地 owner | `SL-S2-06` | Chat、Workspace/File、Process、VCS、Knowledge search/read 保持真实调用；Coding 读写/patch/shell/diff/commit 接入同一 Session 主链；非首批 Wave 3/4 不注册默认模板 | `cargo test --locked -p nomifun-app --lib -- --test-threads=1`；`bun run build:ui`；`bun run dev` 启动烟测 | 2026-09-03 StepFun 代理 live：`gpt-5.5:stepfun-codex` 返回 503；`step-3.7-flash` 返回无可用 content，后续结构探测超过 60 秒；当前仍缺可重复的 Chat/Coding 语义 PASS |
| `SL-S3-08` | closed | 主机 lane | 接入一个真实 MCP Tool 调用 | `SL-S2-06` | v4 `mcp_servers` identity、materialization、MCP package runtime catalog、exact tool/schema 和 credential authority 经 canonical capability；连接失败 typed fail；没有 Gateway/legacy fallback；owner 使用 no-redirect、bounded response 和一次 cleanup | `cargo test --locked -p nomifun-mcp --lib`（250）；`cargo test --locked -p nomifun-app --lib router::agent_wave2_mcp::tests -- --test-threads=1`（7）；`cargo test --locked -p nomifun-app --lib router::agent_wave2_host::tests -- --test-threads=1`（30） | 本机 disposable Streamable HTTP MCP fixture 已执行真实 `tools/call`；OAuth/stdio 仍明确 typed unavailable，不作为本项隐式扩张 |
| `SL-S3-09` | closed | 主机 | 实现精简 SSH read/write/exec/sudo owner primitive | 无 | 真实 host binding；最小 typed command/outcome；path/payload/output/timeout 有界；exec/sudo credential 分离；host-key changed fail；cancel 后回收且不自动重放 | `77bd45279`; `cargo check --locked -p nomi-ssh -p nomifun-ssh`; `cargo test --locked -p nomifun-ssh --lib` | live sshd/sudo 未运行时只记录未运行，不构造 PASS |
| `SL-S3-10` | blocked | 主机 | 完成一个真实 scheduled/automation AgentSession | `SL-S2-07`、`SL-S3-07` | Schedule/Cron/AutoWork/Requirement 复用 canonical Session command/query；计划、执行、取消、恢复不构造 ConversationService/Nomi runtime | 对应 automation crate 定向 tests；一次短周期真实 schedule E2E | 用户人工确认通知或任务结果可作为 UI evidence |
| `SL-S3-11` | blocked | 主机 | 闭合 Remote open/turn/observe/cancel 产品主链 | `SL-S2-03`、`SL-S3-07`、`SL-S3-12` | explicit AgentSession ID；binding/owner/provenance 不漂移；rotate/revoke 不挂起；cancel/delete/cursor/idempotency 明确；无最近会话或旧 selector 旁路 | Remote REST/MCP 定向 tests；真实 `open -> turn -> observe -> cancel` | 需要 installation token 和 live model，均通过现有 secret authority |
| `SL-S3-12` | blocked | 主机 | 按 spike 结果实现最小 Codex-derived Sidecar 与 Model Bridge | `SL-S2-02`、`SL-S2-10`、`SL-S3-07` | Runtime input 含真实 input/route/instructions/context/tools/history；真实 model step 只经 loopback Bridge；binding 独占进程；cancel/dispose 后整棵进程树清理；无 Host 并行文本模型 | `cargo test --locked -p nomifun-codex-runtime --lib`; `cargo test --locked -p nomifun-agent-platform coding_codex --test coding_codex -- --test-threads=1` | 当前缺 exact pinned Sidecar binary 与隔离 live model credential；spike self-test/static review 不能关闭此阻塞 |

## S4：产品 UI

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S4-01` | closed | 主机 lane | 把 Agent 编辑器收缩为产品语言 | `SL-S2-07`、`SL-S2-08` | canonical Agent Settings 已满足名称/用途、模型、能力开关、Workspace/Knowledge/Connector picker、保存和试用；内部 ID/digest/JSON 默认折叠；Save/Test 自动 Preview | Agent Settings 定向测试 `10 passed`；`bun run build:ui` 通过；全量 `tsc` 中 `pages/agentSettings/**` 为 0 条诊断；仓库级 React 19/Arco 类型基线另行记录且未用 `any` 绕过 | 无 |
| `SL-S4-02` | blocked | 主机 | 关闭四条真实用户流程 | `SL-S3-04`～`SL-S3-12`、`SL-S4-01` | 从模板创建；修改并保存；选择资源并试用；Snapshot 不兼容时在新会话继续。Desktop 不崩溃，不要求用户填写 UUID/operation/raw JSON | `bun run dev` 后执行四流程；保留截图、console 和 backend 日志 | 用户可直接人工验收，通常优先于不稳定 Playwright harness |

## S5：三平台、C9 与 RC

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S5-01` | blocked | 主机 | Windows Desktop x64 完成核心候选 | S0-S4 除原生外部项 | package/install/fresh/launch；Chat/Coding/File/Process/VCS/MCP/Browser/Computer/Knowledge/automation/Remote；cancel/crash/process-tree cleanup；无 P0、数据损坏或 secret 泄漏 | 收缩后的 `bun run gate:agent-v2 -- c8-win-pre`，每个 E2E 有独立 deadline | 需要本机 Sidecar、测试 model、MCP 与 Browser/Computer 权限 |
| `SL-S5-02` | external | 外部 | macOS Desktop arm64 原生候选 smoke | `SL-S5-01` | 真 Apple Silicon 上 package/install/launch、critical capability、anchored FS 和 dispose 通过；非 Rosetta；只验证当前候选真实 bytes | macOS arm64 native gate/critical suite | 需要 Apple Silicon、签名/打包环境和 arm64 Sidecar |
| `SL-S5-03` | external | 外部 | Linux Desktop x64 原生候选 smoke | `SL-S5-01` | 真 Linux Desktop x64 上 package/install/launch、Coding、MCP、Browser availability、dispose 通过；Computer 按一期声明明确 available 或 unavailable | Linux Desktop native gate/critical suite | 需要真实 Linux Desktop x64 和 Linux Sidecar |
| `SL-S5-04` | blocked | 主机 | 执行一次性 C9 shutdown 与 Nomi 物理删除 | `SL-S5-01`～`SL-S5-03` | 停止 Nomi admission；取消内部工作；bounded shutdown；kill descendants；unknown 外部 Effect 记 uncertain；删除 Nomi runtime/factory/route/artifact；production/release residual 为 0 | `bun run gate:agent-v2 -- c9-hard-delete` 及 Nomi production/release dependency scan | 进入删除窗口前需要用户确认三平台候选已冻结 |
| `SL-S5-05` | blocked | 主机 | 对同一 Nomi-free RC 完成三平台最终验证并原样提升 Stable | `SL-S5-04` | Windows、macOS arm64、Linux Desktop 对同一 RC bytes 完成 package/install/fresh/critical E2E/lifecycle；release lock/result 可追溯；Stable 不重建 | 三平台 final RC suite；Artifact digest exact-match | 需要当前 Windows 环境、两个对应真实原生环境和发布/签名负责人执行最终 promotion |

## 当前主机并发安排

已完成并记录定向 evidence 的本机基础 lane：

1. `SL-S2-05` SessionEvent/Projection 收缩；
2. `SL-S2-06` 三类 Effect 策略收缩；
3. `SL-S2-07` canonical Compiler；
4. `SL-S2-08` small Snapshot/Fresh-v4 投影；
5. `SL-S2-09` PluginRegistration；
6. `SL-S2-10` official app-server upstream spike；
7. `SL-S3-09` 精简 SSH owner。

当前可实施或可并行准备的本机 lane：

- Core owner lane：`SL-S3-07`，按消费者写集迁移，中央 composition 由集成 Owner 收口；
- MCP lane：`SL-S3-08` 已关闭；后续 OAuth/stdio 扩展不进入本期隐式范围；
- UI lane：`SL-S4-01` 已关闭；仓库级 React/Arco 类型基线不反向阻塞已通过定向验收的
  Agent Settings 产品切片；
- Role Provider lane：`SL-S3-04`、`SL-S3-05` 已关闭；后续只维护真实平台回归；
- Sidecar Runtime lane：`SL-S3-12` 继续实现，但 exact binary/live credential 缺失时只记录
  blocker，不构造 live PASS。

### 2026-09-03 本机 checkpoint

- Rust/App 主线：`cargo test --locked -p nomifun-app --lib -- --test-threads=1`
  `377 passed`；Kernel `18 passed`；Platform sample `2 passed`；Fresh-v4 root `11 passed`。
- MCP：v4 source、host dispatch、owner protocol、strict transport parser 和 lock 校验已合流；
  不读取 legacy `McpServerRow`，不生成 synthetic server/connection ref。
- Remote：修复了无 `requested_session_id` 时由随机 Session ID 参与创建事件 identity
  导致的幂等重放 409；现在同一 Remote open key 会重放同一 Session 及其 terminal
  `open_failed` 结果。`remote_rest_e2e` 两项测试均通过。
- Browser 本机资源检查发现 Chrome `152.0.7977.65` 可用；执行了
  `nomi-browser-engine` 的真实导航和 act fixtures（各 `1 passed`）。随后通过 canonical
  Role/Kernel/Host 链路完成了 `acquire -> observe -> navigate -> act -> render_content
  -> release`，受控测试耗时约 2 秒；alternate Provider parity、owner/lifecycle 和
  boundary scanner 也通过，`SL-S3-04` 已关闭。
- Computer crate 基础测试 `93 passed / 7 ignored`，并在本机真实 Windows Desktop 上
  运行 7 个 ignored 屏幕/输入测试全部通过。随后通过 canonical Role/Kernel/Host 链路完成
  `computer.observe -> computer.input(wait, expected_generation) -> observe`，generation
  单调递增且资源释放/平台关闭通过，`SL-S3-05` 已关闭。macOS TCC 和其他原生平台仍只
  能由对应主机验证。
- Knowledge：rendered URL 现在只接受 typed `BrowserRenderContentPort`，缺少 canonical
  port 时显式失败且不回退 HTTP 或旧 Hub；`nomifun-knowledge` 全量 `315 passed`，
  应用组合已移除旧 `BrowserFetcher -> Hub` 接线。新增
  `bun run check:automation-session-boundary` 审计六类 automation consumer 的旧依赖，
  当前仍明确记录为未安全迁移。
- Gateway/stdio：已物理删除具体 Browser/Computer registry、Gateway capability modules、
  standalone `mcp-computer-stdio` 及其 `ComputerMcpConfig`。`production_bypass_audit`
  和 Browser boundary scanner 均通过；Gateway 保留的 `nomi_*` 工具不再包含 Browser/
  Computer 具体实现，canonical AgentPlatform Role 是唯一入口。
- Desktop：`cargo check/build --locked -p nomifun-desktop --no-default-features` 通过；
  真实 `bun run dev` 已启动 `nomifun-desktop` 窗口，Vite 在 `127.0.0.1:5173` 监听，
  Chrome 加载后显示登录页，未出现 `storage generation must be a canonical lowercase
  UUIDv7 string` 或渲染崩溃页。一次在热重载期间触发的 Rust 编译器
  `0xc0000005 STATUS_ACCESS_VIOLATION` 已记录为构建 harness 障碍；停止并发热重载后
  的单独 build 通过，未继续盲目重试。
- UI：`bun run build:ui` 通过；Agent Settings 定向测试 `10 passed`。全量
  `bun run typecheck` 当前仍有 `446` 条 React 19/Arco 2.66.15 依赖类型诊断
  （主要为 `Modal/Trigger` JSX 与回调上下文），Agent Settings 本身无诊断；
  已清理唯一独立的未使用变量，未用 `any` 绕过依赖基线。
- Automation 审计确认 Cron、AutoWork、AgentExecution 生产消费者仍经旧
  `ConversationService`/Nomi runtime，不能把 typed delegator 单测当作 canonical Session close。
- 当前所有开发、合流和验证继续在本机进行；没有机器 2 Prompt、handoff、压缩包或跨机 attestation。

所有 lane 都在当前主机使用互斥路径写集；不得同时编辑同一文件或争用共享数据库、固定
端口、Cargo 构建目录和进程树。`Cargo.lock`、中央 Compiler/Snapshot、Fresh-v4 schema、
Gate 和 GLOBAL TODO 由集成 Owner 串行修改。每个 lane 只记录 changed paths、验证命令、
未运行项和阻塞原因，不生成机器专用 Prompt、manifest、result template、远端 SHA、
handoff 或跨机 attestation。

## 推荐顺序

1. 主机按互斥写集推进 `SL-S3-07`；MCP `SL-S3-08` 与 Agent Settings `SL-S4-01`
   已完成，中央文件串行合流。
2. 单 Compiler、小 Snapshot、PluginRegistration 和 Role 合同稳定后，按
   `SL-S3-01 -> SL-S3-02 -> SL-S3-03` 串行落主链。
3. Browser 与 Computer first-party Provider 在 RoleDispatcher 稳定后可并行实现，随后统一清除旁路。
4. 核心 owner、MCP、automation、Remote、Sidecar 以不争抢中央 composition 文件的批次合流。
5. S4 四条 UI 流程通过后才形成 Windows 候选；测试障碍转人工，不重复盲跑。
6. Windows 完整闭环后，冻结候选供 macOS arm64 与 Linux Desktop 外部原生验证；验证完成后执行 C9。
7. C9 后对同一 Nomi-free RC 做三平台最终验证，Stable 只提升已验证 bytes。

## 阶段退出条件

- **S0 完成**：本文审查提交，单机多并发规则生效，旧 84 项不再驱动施工。
- **S1 完成**：Wave 3/4 都有明确 keep/revert 结果，保留代码都有真实用户价值。
- **S2 完成**：三个 P0 关闭；单 Compiler、小 Snapshot、三类 Effect 生效；Sidecar 最小协议
  基于真实 upstream spike，而不是历史假设。
- **S3 完成**：Browser/Computer first-party 与 alternate fixture 走同一 Role seam；核心用户
  owner、MCP、SSH、automation、Remote 和 Codex-derived Runtime 可真实执行；具体实现旁路为 0。
- **S4 完成**：四条用户流程可从 `bun run dev` 正常验收，普通用户不接触内部标识和 JSON。
- **S5 完成**：Windows、macOS arm64、Linux Desktop 的同一 Nomi-free RC 通过，C9 已物理删除
  Nomi，具备执行 05 `S6 Stable` 原样提升的条件。

macOS x64、Linux Headless、Wave 3/4 非核心业务全覆盖、Knowledge 高级写入/embedding/rerank、
所有 Channel/Robot/Customer 场景、性能 benchmark 和长观察窗口不阻塞首个 Stable。未来实际
宣称支持相应平台或功能时，再建立独立、真实、可验证的交付任务。
