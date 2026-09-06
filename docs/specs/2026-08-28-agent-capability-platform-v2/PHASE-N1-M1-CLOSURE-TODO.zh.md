# Plugin N1 / MiniApp M1 Windows 实施台账

> 启动日期：2026-09-06
>
> 设计合同：`06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md`
>
> 前置合同：`05-system-capability-replacement-foundation.zh.md`
>
> 当前结论：AP-0～AP-7 与一期 Windows C8 已通过。用户已授权在 Windows 主机继续
> 完成 Plugin N1 和 MiniApp M1；macOS arm64、Linux Desktop x64 统一延后到全部
> Windows 开发与候选验证完成后交接。手机模式不属于 `nomifun-desktop` 范围。

本文是 06 的唯一实时执行台账。06 保存产品与架构合同，GLOBAL TODO 保存一期 S0-S5；
二期状态不得回填成一期完成度，也不得用旧 Extension/MiniApp 的代码量冒充 N1/M1 进度。

## 状态规则

| 状态 | 含义 |
| --- | --- |
| `open` | 前置满足，可立即实施 |
| `in-progress` | 当前主机正在实施，写集已有 owner |
| `blocked` | 等待本台账中的代码依赖，不是等待外部机器 |
| `pending-validation` | 实现已形成，等待 Windows Gate/产品验收 |
| `external` | 仅真实 macOS/Linux 原生环境可关闭 |
| `deferred` | 明确不进入 N1/M1 |
| `closed` | 完成定义和最小证据均满足 |

## 执行约束

1. 大胆 clean cut：新 Plugin/MiniApp 主链可用后物理删除旧 Extension 与旧 MiniApp；
   不双读、双写、alias、fallback 或保留隐藏入口。
2. `Cargo.toml`、`Cargo.lock`、根 `package.json`/`bun.lock`、App composition、
   Fresh-v4 schema、generated contracts、Router/Sider、Gate 与本文由集成 Owner 串行修改。
3. 并行 lane 必须互斥写集；不得争用固定端口、共享 DB、Cargo release build 或安装进程。
4. Chat Dev/模型 E2E 固定使用 Windows Credential Manager 中的 StepFun Coding Plan
   `step-3.7-flash`；secret 不进入源码、日志、argv、fixture、文档或 Git。
5. Windows 只验证 Desktop x64/NSIS/accessibility，不运行手机视口。
6. macOS/Linux 在 `RC-WIN-01` 关闭前不领取开发任务、不产生候选结论。

## 当前快照

| 分类 | 数量 | 项目 |
| --- | ---: | --- |
| 已关闭 | 1 | `W0-01` |
| 当前可实施 | 2 | `N1-0-01`、`N1-X-01` |
| 依赖阻塞 | 27 | 其余 N1/M1 Windows 项与最终合流 |
| 外部原生 | 2 | `RC-MA-01`、`RC-LD-01` |
| 明确延后 | 2 | Marketplace/远程分发、第二 Runtime |

## W0：一期交接

| ID | 状态 | 目标 | 完成定义 | Evidence |
| --- | --- | --- | --- | --- |
| `W0-01` | closed | 关闭一期 Windows C8 | 候选 package/install/fresh/launch、真实 StepFun、Browser/Computer、Remote、cleanup、release lock 全部通过 | candidate `0bac72da4ebb62f6a0f183a1285065c88aa684a4`; Host `555a4560...`; NSIS `d0f22840...`; lock `54e39e87...` |

## N1-0：机器合同冻结

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-0-01` | open | 集成 Owner；全仓只读扫描、06/本文 | repo dependency/product residual scan，冻结删除顺序和真实消费者 | `W0-01` | dependency report；旧 `/api/presets` 仍为 0 |
| `N1-0-02` | blocked | Contract lane；`nomifun-agent-contracts/src/plugin_*`、`contracts/plugin-n1/**`、`nomifun-api-types/src/plugin*` | 冻结 `plugin-package-v1`、Host IPC、Runtime fingerprint、Credential slot、Project/Candidate/TestReceipt/Apply/Share 合同 | `N1-0-01` | canonical serialization/schema tests |
| `N1-0-03` | blocked | Kernel lane；`nomifun-agent-kernel/src/{materialize,registry,compiler,plugin,error}.rs` | Snapshot/operation 精确锁定 mount/contribution/artifact，不依赖无关全局 generation | `N1-0-02` | unrelated catalog change 不破坏旧 Snapshot |
| `N1-0-04` | blocked | 集成 Owner；validation contract/Gate | 建立 Windows N1/M1 stage、required/optional cell 和最终 cohort 合同 | `N1-0-02` | Gate self-test；无 candidate SHA 自引用 |

## N1-1：Node Foundation

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-1-01` | blocked | Runtime lane；新 `nomifun-js-runtime/**` | Node PATH/手工/managed LTS probe、下载确认、fingerprint 与全局试切换 | `N1-0-02` | probe/selection/fail-closed tests |
| `N1-1-02` | blocked | Runtime lane；新 JS Host package/crate | lazy shared Extension Host、private IPC/Hello、watchdog、whole-tree cleanup、late-result fence | `N1-1-01` | demand=0 process=0；crash/restart/cleanup |
| `N1-1-03` | blocked | Kernel+Runtime 边界 | Tool/Context/Resource/Role Provider 的 Node proxy exports | `N1-0-03`,`N1-1-02` | exact lock invoke；无 Rust/Node 双 Registry |

## N1-2：Package 与数据生命周期

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-2-01` | blocked | DB lane；migration 067+、新 repository | Artifact/Project/Candidate/current/previous/Mount/Operation/retained data schema | `N1-0-02` | fresh DB、migration lineage、restart |
| `N1-2-02` | blocked | Plugin platform lane | Config、Credential slot binding、KV/CAS、stable `dataDir`、owner mutation lock | `N1-2-01`,`N1-1-02` | namespace/CAS/secret rotation tests |
| `N1-2-03` | blocked | Plugin platform lane | staging/containment/digest/install/replace/restore/uninstall/delete-data | `N1-2-01`,`N1-2-02` | failed replace keeps current；delete resumable |

## N1-3：Catalog 与消费者

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-3-01` | blocked | Catalog lane | ManagedLocal Package/Mount materialization、五类 Contribution、provenance/availability | `N1-0-03`,`N1-1-03`,`N1-2-03` | duplicate hard-fail；consumer filtering |
| `N1-3-02` | blocked | Nomi consumer lane | 用动态 action schema/Kernel invoke 替换 Nomi 硬编码 Capability→Tool 表 | `N1-3-01` | AgentPreset compile/invoke/impact |
| `N1-3-03` | blocked | 非 Agent consumer lane | 一个共享 Capability 与一个 non-Agent-only reference contribution | `N1-3-01` | Agent+非 Agent exact lock；picker filter |

## N1-4：Authoring 与 Self-Evolution

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-4-01` | blocked | Authoring lane；新 JS authoring/SDK | Source Store、JS/TS scaffold、pure-JS dependency exact lock、fixed packer/Build Host | `N1-0-02`,`N1-1-02`,`N1-2-01` | reproducible artifact；cancel cleanup |
| `N1-4-02` | blocked | Plugin project lane | single Ready、Candidate Test、impact、manual/compatible-idle Apply、Retry/Restore | `N1-2-03`,`N1-3-01`,`N1-4-01` | stale base/busy/breaking/no auto rollback |
| `N1-4-03` | blocked | SDK/CLI lane | Plugin SDK、Share Bundle/prebuilt import/export、本地 CLI | `N1-4-02` | same application service；secret-free bundle |

## N1-X：旧 Extension 删除与产品 UI

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-X-01` | open | Skill lane；新 `nomifun-skill-library/**` | 把 `skill_service`、builtin skills、Skill market 从 `nomifun-extension` 抽为独立 owner | `W0-01` | 现有 Skill 用户流程/测试不回退 |
| `N1-X-02` | blocked | demolition lane；`nomifun-extension/**` 及消费者 | 删除旧 Extension loader/registry/hub/hot reload/permissions/settings/webui/agent/theme 路径 | `N1-X-01`,`N1-3-03`,`N1-4-03` | `/api/extensions/*`、Hub、`nomi-extension.json` 生产可达性为 0 |
| `N1-U-01` | blocked | UI lane；新 `pages/plugins/**`、Runtime Manager | Plugin Library/Workshop/配置/诊断、Node Runtime Manager；MCP 页面只保留 MCP | `N1-2-03`,`N1-4-02` | Desktop product tests/build/a11y |
| `N1-V-01` | blocked | 集成 Owner | Windows N1 contract/integration/fault/product/NSIS candidate | 所有 N1 项 | authored JS/TS → Test → Apply → invoke → Restore |

## M1：Full-stack MiniApp

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `M1-0-01` | blocked | MiniApp DB/domain lane | 全新 Product/Project/Ready/Active/Previous 数据根，不读旧 `miniapps` | `N1-V-01` | fresh schema；旧表无生产读 |
| `M1-0-02` | blocked | MiniApp release lane | UI-only `miniapp-release-v1`、manual/pure-UI auto Publish、Rollback、Host KV | `M1-0-01`,`N1-4-01` | UI-only 全程 Node process=0 |
| `M1-1-01` | blocked | Service/Bridge lane | 单 `main.mjs`、dedicated Host、on-demand/continuous、MessageChannel epoch fence | `M1-0-02`,`N1-1-02` | old port callback rejected；one App crash isolation |
| `M1-1-02` | blocked | Managed data lane | UI/Service KV、Files、Private SQLite、additive migration ledger | `M1-1-01` | owner namespace/SQL boundary/migration |
| `M1-2-01` | blocked | Lifecycle lane | Enable/Disable/Trash/Restore/Permanent Delete、Share/Backup Import-as-new | `M1-1-02` | resumable delete；no plaintext credential |
| `M1-U-01` | blocked | UI lane；整体重写 `pages/miniApps/**` | Library/Workshop/Surface，删除 Guid/Conversation 旧 MiniApp 模式 | `M1-0-02`,`M1-1-01` | real Desktop workflow/build/a11y |
| `M1-V-01` | blocked | 集成 Owner | Windows M1 contract/integration/fault/product/NSIS candidate | 所有 M1 项 | UI-only + Service representative lifecycle |

## 最终候选与外部验证

| ID | 状态 | 目标 | 依赖 | 完成定义 |
| --- | --- | --- | --- | --- |
| `RC-WIN-01` | blocked | 冻结全部 Windows 开发的最终 source cohort | `N1-V-01`,`M1-V-01` | final NSIS、StepFun、Plugin/MiniApp installed-app smoke、release lock/result |
| `RC-MA-01` | external | macOS arm64 required 原生验证 | `RC-WIN-01` | 同 cohort package/install/runtime/Plugin/MiniApp/cleanup |
| `RC-LD-01` | external | Linux Desktop x64 required 原生验证 | `RC-WIN-01` | 同 cohort Desktop/CLI/runtime/Plugin/MiniApp/cleanup |
| `RC-MERGE-01` | blocked | required 三平台原样提升 | `RC-WIN-01`,`RC-MA-01`,`RC-LD-01` | 同 source/input/digest，Stable 不重建 |

## 明确延后

- Marketplace、publisher、URL/Git 安装、远程 Registry 自动更新。
- 第二 Runtime、Bun Runtime provider、多 Runtime 并行。
- macOS x64、Linux Headless、手机模式。
