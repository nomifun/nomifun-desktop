# Agent Capability Platform v2 一期精简闭合 TODO

> 盘点日期：2026-09-02
>
> 基线分支：`rf/agent-capability-platform-v2`
>
> 权威来源：`05-system-capability-replacement-foundation.zh.md`
>
> 当前结论：一期止损已生效，但 S1-S5 尚未完成；不得宣称 C8、C9、Stable
> 或 Browser/Computer 可替换主链已经完成。

本文是 05 发布后的唯一一期执行台账。旧版 84 个 `INF/W/LEG/SCN/TST/REL`
ID 从现在起只作为历史审计索引，不再是一期必须逐项关闭的阻断清单，也不得继续用
“81 个 action-bearing Capability 是否全部有 owner”、旧 residual 数量或五平台笛卡尔积
衡量一期完成度。

一期只追踪 05 要求的 S0-S5：先停止错误扩张和审计普通 revert，再关闭三个 P0、
单 Compiler、小 Snapshot、三类 Effect、Sidecar upstream spike，随后完成
Browser/Computer Role seam、真实核心 owner、四条 UI 用户流程、Windows、macOS arm64、
Linux Desktop 和一次性 C9 clean cut。

## 状态与执行规则

| 状态 | 含义 |
| --- | --- |
| `open` | 当前 owner 可以直接实施，不需要等待其他 TODO |
| `blocked` | 必须先关闭依赖项；不得通过兼容层、mock 或 synthetic PASS 绕过 |
| `external` | 需要另一真实原生主机、签名环境或用户提供的 live 资源 |
| `pending-validation` | 实施内容已形成，但尚待审查、提交或指定验证 |
| `closed` | 完成定义已满足，并有提交或可复查的最小 evidence |

执行约束：

1. 05 与本文冲突时以 05 为准；01-04、旧 GLOBAL TODO、旧 Machine Prompt 只作历史背景。
2. 不使用 reset、force-push 或历史重写；revert 必须使用普通提交，并先检查真实消费者。
3. 每个任务只实现一个实际闭环；需要第二份事实、新 coordinator、新全局 digest 或新状态机
   时先停止并重新核对 05。
4. 非首批 Capability 可以保持明确 unavailable，但不得返回 metadata-only success，也不得
   阻塞核心 Stable。
5. API key、token、私钥、主机地址和签名 secret 不进入源码、文档、fixture、日志或 Prompt。
6. 测试遇到环境或 harness 障碍时记录首个完整失败、停止盲目重试，并提供人工替代步骤。
7. 二期 `06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md` 不属于本台账，
   不得在一期实现、提交或引用为已冻结合同。

## 历史进度保留

以下是真实已完成或可复用的功能，不因止损而删除，但也不自动关闭后续集成任务：

| Commit | 保留内容 | 后续处置 |
| --- | --- | --- |
| `099893cc` | Fresh-v4 storage generation 启动修复 | 保留，继续作为 `bun run dev` 基线 |
| `745fabfa` | binding-backed `knowledge.search/read` owner | 保留真实 owner |
| `280841b3` | Agent Preset KnowledgeBase picker | 保留并纳入 S4 用户流程 |
| `5d691824` | anchored Knowledge 文件访问 | 保留基本 containment；不继续扩大极端本机攻击证明 |
| `3f835174`、`c6503a23` | canonical AgentSession command/query ServiceKey 及 core service package host 测试适配 | 保留单一 Session authority |
| `8aade375` | 真实 local/file `vcs.push` owner | 保留 owner；按三类 Effect forward 简化 |
| `b58a0f92` | fork cursor 修复 | 保留产品语义 |
| `dd07b937` | Remote public route 改用 Session command/query ports | 保留；Remote auth fence 与真实 Runtime 仍未关闭 |
| `efbcb598`、`23e039ff`、`1a547f3a`、`f46cc017` | Knowledge 的 Windows/macOS 工程验证 | 保留为工程记录，不冒充最终候选 native PASS |

已按止损结论处理的历史实现：

- `d1acccf6` Wave 3 批量 typed contract：已由 `2ad8ca12` 普通 revert。
- `765d1953` Wave 4 通用 Effect/receipt contract：已由 `8f4ba1d9` 普通 revert。
- 旧 C8/C10 cohort、handoff、fixture digest、五格 evidence 和 residual 分类实现：
  只保留修复 P0、三平台 RC 和发布追溯真正需要的最小部分。

## 汇总

| 阶段 | closed | open | blocked | external | pending-validation | 合计 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| S0 止损发布 | 3 | 0 | 0 | 0 | 0 | 3 |
| S1 Revert/keep 审计 | 3 | 0 | 0 | 0 | 0 | 3 |
| S2 P0 与基础收缩 | 3 | 7 | 0 | 0 | 0 | 10 |
| S3 Role seam 与核心 owner | 0 | 4 | 8 | 0 | 0 | 12 |
| S4 产品 UI | 0 | 1 | 1 | 0 | 0 | 2 |
| S5 三平台、C9 与 RC | 0 | 0 | 3 | 2 | 0 | 5 |
| **总计** | **9** | **12** | **12** | **2** | **0** | **35** |

旧台账 84 项现已收敛为 35 项。任务数量不是质量指标；只有完成定义和最小验证满足后
才能修改状态。

## S0：止损发布

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S0-01` | closed | 主机 | 发布 05 并停止旧 84 项驱动的扩张 | 无 | 05 独立提交，明确覆盖旧 Gate、Effect、平台矩阵和 TODO 口径 | `git show --stat d6de5170` | 无 |
| `SL-S0-02` | closed | 主机 | 用本文替换旧 84 项阻断台账 | `SL-S0-01` | 只保留 S0-S5 stable ID、状态、owner、依赖、完成定义、最小测试和人工输入；统计自洽 | `df4bdf56`; `git diff --check -- docs/specs/2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md` | 无 |
| `SL-S0-03` | closed | 主机 | 把机器 2 收缩为唯一精简 SSH lane | `SL-S0-01` | Prompt 删除绝对原子覆盖、通用 uncertain/reconcile、中央 Effect journal 和长期旧 API 兼容要求 | `git show --stat b13a8164` | 机器 2 开始前需拉取包含 05 和新 Prompt 的基线 |

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
| `SL-S2-04` | open | 主机 | 简化 D-024 delete/dispose | `SL-S1-03` | 删除调用者填写的 `ZeroOutstandingProof`；使用真实 `RuntimeDisposeReport`；`deleting` 重启后幂等完成 tombstone | `cargo test -p nomifun-agent-session delete --lib -- --test-threads=1` | 无 |
| `SL-S2-05` | open | 主机 | 收缩 SessionEvent 与 Projection | `SL-S2-04` | Event Log 保留唯一语义事实；Projection 不复制完整 `events[]`；正常完成只持久化最终 assistant message，中断最多一份 bounded partial | `cargo test -p nomifun-agent-session projection --lib` | 无 |
| `SL-S2-06` | open | 主机 | 把 Effect 生命周期收敛为三种策略 | `SL-S1-01`、`SL-S1-02` | 仅保留 `read_only`、`managed_effect`、`external_uncertain_effect`；本地操作使用事务/CAS/原子文件；外部 unknown 不自动 retry；删除 Wave 级通用 journal/coordinator | `cargo test -p nomifun-agent-session effect --lib -- --test-threads=1` | live 外部 Effect 只在可丢弃 sandbox 验证 |
| `SL-S2-07` | open | 主机 | 合并为一个 canonical Compiler | `SL-S1-03` | Preview/Save/Test 共用同一纯函数 Compiler；Session Open 读取已保存 Snapshot，只做当前执行兼容检查；删除第二份 closure/digest 算法 | Compiler 定向 unit tests；同一输入的 Preview/Save/Test Snapshot digest 相同 | 无 |
| `SL-S2-08` | open | 主机 | 缩小 Snapshot、CapabilitySelection 和 Fresh-v4 投影 | `SL-S2-07` | Snapshot 只锁实际 Capability/Provider/Tool/Model/resource/runtime 闭包；删除未执行 selection 字段和只写不读的重复投影；fresh-v4 fixture 可双启动 | `cargo test -p nomifun-v4-root -- --test-threads=1; cargo test -p nomifun-agent-kernel compiler --lib` | 无 |
| `SL-S2-09` | open | 主机 | 简化 PluginRegistration | `SL-S2-07` | Manifest 是声明事实源；registration metadata 从真实 handler/service exports 派生；保留 namespace、schema、typed dependency、duplicate/cycle 和 cleanup | `cargo test -p nomifun-agent-kernel materialize --lib` | 无 |
| `SL-S2-10` | open | 主机 | 完成 Codex official app-server upstream spike并冻结最小 Sidecar 协议 | 无 | 验证 initialize/version、thread/turn/cancel/event、Host-managed Tool 和进程关闭；只在 upstream 无法提供必要 pre-effect seam 时提出一个窄 patch；不预设三项自定义 RPC | 在 pinned upstream 上运行 app-server 协议 smoke，并提交 spike 记录、调用 trace 和 patch/no-patch 结论 | 如需 live model，使用本机 secret store 注入测试 credential |

## S3：Role seam 与核心 owner

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S3-01` | open | 主机 | 冻结 Browser/Computer versioned Role 合同 | `SL-S2-09` | `ExecutionRoleId`、Role Contract、source-neutral Provider contribution、required/optional member、typed Context/Resource exports 只有一套 canonical Rust/schema 定义 | `cargo test -p nomifun-agent-contracts role --lib; cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check` | 无 |
| `SL-S3-02` | blocked | 主机 | 实现 installation binding、Revision override、Resolver 和 Snapshot exact lock | `SL-S2-07`、`SL-S2-08`、`SL-S3-01` | override 优先、缺省继承 installation default；精确 Provider/contract/contribution/resource 进入 Snapshot digest；缺失明确失败且不 fallback | Compiler/Resolver tests：first-party 与 alternate fixture 生成不同且自洽的 Snapshot | 无 |
| `SL-S3-03` | blocked | 主机 | 实现单一 RoleDispatcher 与 Tool/Context/Resource runtime seam | `SL-S3-02` | Kernel 第一次路由直接选 frozen Provider Mount；使用 Provider config/state/service/resource；不 façade 二次调用、不重选、不 retry/fallback | `cargo test -p nomifun-agent-kernel role_dispatch --lib` | 无 |
| `SL-S3-04` | blocked | 主机 | 第一方 Browser dogfood 同一 Role 主链 | `SL-S3-03` | observe/navigate/act 和 hidden `browser.render_content` 经同一 Provider lock；保留 owner/lane/close/process cleanup；Provider 平台约束不写死在 façade | Browser owner/lifecycle tests；alternate Provider parity test | Browser live E2E 需要可访问测试页 |
| `SL-S3-05` | blocked | 主机 | 第一方 Computer/A11y dogfood 同一 Role 主链 | `SL-S3-03` | observe/input 基线和可选 launch/a11y 经 exact Provider；按 target resource 串行；observation generation 过期 typed fail；无具体 Registry 旁路 | Computer serialization/generation/platform-unavailable tests | Windows/macOS input 权限由原生测试用户授予 |
| `SL-S3-06` | blocked | 主机 | 删除 Browser/Computer production concrete bypass | `SL-S3-04`、`SL-S3-05` | Wave 2 不再 unavailable；Knowledge hidden render、Gateway、stdio、v4/Codex/automation 只走 canonical route；Nomi-only allowlist 不增长并等待 C9 | production dependency scan；Knowledge render、Gateway、stdio 代表性 integration tests | 无 |
| `SL-S3-07` | open | 主机 | 收口真实核心本地 owner | `SL-S2-06` | Chat、Workspace/File、Process、VCS、Knowledge search/read 保持真实调用；Coding 读写/patch/shell/diff/commit 接入同一 Session 主链；非首批 Wave 3/4 不注册默认模板 | `cargo test -p nomifun-app agent_platform_host --lib; bun run dev` | Chat/Coding live 验证需要已配置 model/provider |
| `SL-S3-08` | open | 主机 | 接入一个真实 MCP Tool 调用 | `SL-S2-06` | MCP server/resource binding、exact tool/schema 和 credential authority 经 canonical capability；连接失败 typed fail；没有 Gateway/legacy fallback | `cargo test -p nomifun-mcp --lib; cargo test -p nomifun-app mcp --lib -- --test-threads=1` | 需要一个可丢弃 MCP 测试服务 |
| `SL-S3-09` | open | 机器2 | 实现精简 SSH read/write/exec/sudo owner primitive | `SL-S0-03` | 真实 host binding；最小 typed command/outcome；path/payload/output/timeout 有界；exec/sudo credential 分离；host-key changed fail；cancel 后回收且不自动重放 | `cargo check --locked -p nomi-ssh -p nomifun-ssh; cargo test --locked -p nomifun-ssh --lib` | live sshd/sudo 不可用时只记录未运行，不构造 PASS |
| `SL-S3-10` | blocked | 主机 | 完成一个真实 scheduled/automation AgentSession | `SL-S2-07`、`SL-S3-07` | Schedule/Cron/AutoWork/Requirement 复用 canonical Session command/query；计划、执行、取消、恢复不构造 ConversationService/Nomi runtime | 对应 automation crate 定向 tests；一次短周期真实 schedule E2E | 用户人工确认通知或任务结果可作为 UI evidence |
| `SL-S3-11` | blocked | 主机 | 闭合 Remote open/turn/observe/cancel 产品主链 | `SL-S2-03`、`SL-S3-07`、`SL-S3-12` | explicit AgentSession ID；binding/owner/provenance 不漂移；rotate/revoke 不挂起；cancel/delete/cursor/idempotency 明确；无最近会话或旧 selector 旁路 | Remote REST/MCP 定向 tests；真实 `open -> turn -> observe -> cancel` | 需要 installation token 和 live model，均通过现有 secret authority |
| `SL-S3-12` | blocked | 主机 | 按 spike 结果实现最小 Codex-derived Sidecar 与 Model Bridge | `SL-S2-02`、`SL-S2-10`、`SL-S3-07` | Runtime input 含真实 input/route/instructions/context/tools/history；真实 model step 只经 loopback Bridge；binding 独占进程；cancel/dispose 后整棵进程树清理；无 Host 并行文本模型 | `cargo test -p nomifun-codex-runtime --lib; cargo test -p nomifun-agent-platform coding_codex --test coding_codex -- --test-threads=1` | live E2E 需要构建出的 Sidecar 和测试 model credential |

## S4：产品 UI

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S4-01` | open | 主机 | 把 Agent 编辑器收缩为产品语言 | `SL-S2-07`、`SL-S2-08` | 默认只展示名称/用途、模型、能力开关、Workspace/Knowledge/Connector picker、保存和试用；内部 ID/digest/JSON 默认折叠；Save/Test 自动 Preview | `bun run typecheck; bun run test -- <相关 UI 定向测试>` | 无 |
| `SL-S4-02` | blocked | 主机 | 关闭四条真实用户流程 | `SL-S3-04`～`SL-S3-12`、`SL-S4-01` | 从模板创建；修改并保存；选择资源并试用；Snapshot 不兼容时在新会话继续。Desktop 不崩溃，不要求用户填写 UUID/operation/raw JSON | `bun run dev` 后执行四流程；保留截图、console 和 backend 日志 | 用户可直接人工验收，通常优先于不稳定 Playwright harness |

## S5：三平台、C9 与 RC

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S5-01` | blocked | 主机 | Windows Desktop x64 完成核心候选 | S0-S4 除原生外部项 | package/install/fresh/launch；Chat/Coding/File/Process/VCS/MCP/Browser/Computer/Knowledge/automation/Remote；cancel/crash/process-tree cleanup；无 P0、数据损坏或 secret 泄漏 | 收缩后的 `bun run gate:agent-v2 -- c8-win-pre`，每个 E2E 有独立 deadline | 需要本机 Sidecar、测试 model、MCP 与 Browser/Computer 权限 |
| `SL-S5-02` | external | 外部 | macOS Desktop arm64 原生候选 smoke | `SL-S5-01` | 真 Apple Silicon 上 package/install/launch、critical capability、anchored FS 和 dispose 通过；非 Rosetta；只验证当前候选真实 bytes | macOS arm64 native gate/critical suite | 需要 Apple Silicon、签名/打包环境和 arm64 Sidecar |
| `SL-S5-03` | external | 外部 | Linux Desktop x64 原生候选 smoke | `SL-S5-01` | 真 Linux Desktop x64 上 package/install/launch、Coding、MCP、Browser availability、dispose 通过；Computer 按一期声明明确 available 或 unavailable | Linux Desktop native gate/critical suite | 需要真实 Linux Desktop x64 和 Linux Sidecar |
| `SL-S5-04` | blocked | 主机 | 执行一次性 C9 shutdown 与 Nomi 物理删除 | `SL-S5-01`～`SL-S5-03` | 停止 Nomi admission；取消内部工作；bounded shutdown；kill descendants；unknown 外部 Effect 记 uncertain；删除 Nomi runtime/factory/route/artifact；production/release residual 为 0 | `bun run gate:agent-v2 -- c9-hard-delete` 及 Nomi production/release dependency scan | 进入删除窗口前需要用户确认三平台候选已冻结 |
| `SL-S5-05` | blocked | 主机 | 对同一 Nomi-free RC 完成三平台最终验证并原样提升 Stable | `SL-S5-04` | Windows、macOS arm64、Linux Desktop 对同一 RC bytes 完成 package/install/fresh/critical E2E/lifecycle；release lock/result 可追溯；Stable 不重建 | 三平台 final RC suite；Artifact digest exact-match | 需要三台真实主机和发布/签名负责人执行最终 promotion |

## 机器 2 唯一 Lane

机器 2 当前只领取 `SL-S3-09`，不再领取 MCP、Browser、Computer、Wave 3/4、
Sidecar 或 native Gate。允许写集仅为：

- `crates/backend/nomifun-ssh/src/**`
- `crates/backend/nomifun-ssh/tests/**`
- `crates/shared/nomi-ssh/src/**`
- `crates/shared/nomi-ssh/tests/**`

机器 2 不修改 `Cargo.lock`、canonical contracts、Fresh-v4 schema、Wave domain、app host、
Gateway、Gate、manifest 或本文。完成后以独立分支普通 push，回传 base SHA、commit SHA、
changed paths、每条验证命令、未运行项和主机接线说明。主机在机器 2 开发期间继续 S1、S2、
S3 非 SSH 工作，不等待 SSH lane 才推进。

## 推荐顺序

1. 以已完成的 Wave 3/4 普通 revert 为基线，立即完成 `SL-S1-03`，同时并行执行三个 P0：
   `SL-S2-01`、`SL-S2-02`、`SL-S2-03`。
2. 并行推进 `SL-S2-04`～`SL-S2-10`；机器 2 独立推进 `SL-S3-09`。
3. 单 Compiler、小 Snapshot 和 Role 合同稳定后，按
   `SL-S3-01 -> SL-S3-02 -> SL-S3-03` 串行落主链。
4. Browser 与 Computer first-party provider 可并行实现，随后统一清除旁路。
5. 核心 owner、MCP、automation、Remote、Sidecar 以不争抢中央 composition 文件的批次合流。
6. S4 四条 UI 流程通过后才形成 Windows 候选；测试障碍转人工，不重复盲跑。
7. Windows 完整闭环后，把同一候选交给 macOS arm64 与 Linux Desktop；两者完成后执行 C9。
8. C9 后对同一 Nomi-free RC 做三平台最终验证，Stable 只提升已验证 bytes。

## 阶段退出条件

- **S0 完成**：本文审查提交，机器 2 使用精简 Prompt，旧 84 项不再驱动施工。
- **S1 完成**：Wave 3/4 都有明确 keep/revert 结果，保留代码都有真实用户价值。
- **S2 完成**：三个 P0 关闭；单 Compiler、小 Snapshot、三类 Effect 生效；Sidecar 最小协议
  基于真实 upstream spike，而不是历史假设。
- **S3 完成**：Browser/Computer first-party 与 alternate fixture 走同一 Role seam；核心用户
  owner、MCP、SSH、automation、Remote 和 Codex-derived Runtime 可真实执行；具体实现旁路为 0。
- **S4 完成**：四条用户流程可从 `bun run dev` 正常验收，普通用户不接触内部标识和 JSON。
- **S5 完成**：Windows、macOS arm64、Linux Desktop 的同一 Nomi-free RC 通过，C9 已物理删除
  Nomi，Stable 原样提升同一制品。

macOS x64、Linux Headless、Wave 3/4 非核心业务全覆盖、Knowledge 高级写入/embedding/rerank、
所有 Channel/Robot/Customer 场景、性能 benchmark 和长观察窗口不阻塞首个 Stable。未来实际
宣称支持相应平台或功能时，再建立独立、真实、可验证的交付任务。
