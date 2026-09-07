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
| 已关闭 | 14 | `W0-01`、`N1-0-01`～`N1-0-05`、`N1-1-01`～`N1-1-03`、`N1-2-00`～`N1-2-02`、`N1-3-03`、`N1-X-01` |
| 正在实施 | 4 | `N1-2-03`、`N1-3-01`、`N1-4-01`、`N1-U-01` |
| 已解锁待领取 | 0 | 无 |
| 依赖阻塞 | 14 | 其余 N1/M1 Windows 项与最终合流 |
| 外部原生 | 2 | `RC-MA-01`、`RC-LD-01` |
| 明确延后 | 2 | Marketplace/远程分发、第二 Runtime |

## 2026-09-06 实施记录

1. `N1-0-01` 已完成全仓依赖与产品残留扫描：
   - 旧 `/api/presets` 生产可达性仍为 `0`；
   - 旧 Extension 非 Skill 主链仍有 App、Channel、Gateway、UI 等真实消费者，必须等新
     Plugin 主链可用后按 UI → Gateway/Channel → App composition → crate 的顺序删除；
   - 旧 MiniApp REST/UI/DB/Conversation 路径仍完整生产可达，且当前
     `miniapp.read/edit/publish/serve` 没有真实 M1 owner，不能计入二期完成度。
2. `N1-X-01` 已由提交 `610b59007` 物理抽离：
   `nomifun-skill-library` 独立拥有 Skill service/routes/assets/market，所有直接消费者已
   切换，`nomifun-extension` 不保留 Skill re-export、alias 或 fallback。
3. `N1-0-02` 已冻结 Plugin N1 合同：
   - `plugin-package-v1` 直接嵌入唯一 canonical `PackageManifest`；
   - Package entrypoint 是 `in_process | javascript` 联合类型，Rust/Node 共用一个
     Materializer/Registry；
   - Host role-specific Hello/wire direction/request/response/generation fence、Node
     probe/switch、Credential/KV/dataDir、Project/Ready/TestReceipt、manual/authorized
     auto Apply、Restore、Share、Operation 与 cohort 合同已进入 generator。
4. `N1-0-03` 已让 `CapabilityManifest` 直接拥有稳定 `contribution_id`，并使
   Revision/Snapshot/Invoke 精确锁定 contribution、Mount、Source 与 Artifact。
   无关 Registry publication 不再误杀旧 Snapshot，任一 exact provenance 漂移均
   fail closed 且不 fallback。
5. `N1-0-04` 已建立 `gate:plugin-n1`：Plugin/MiniApp 合同 digest、stage/scope、
   required/optional cells、clean HEAD、result/cohort 与 Windows host 防冒充均有一套
   fail-closed Gate；Candidate/RC 未实现 check 只会返回 blocked，不会伪造 PASS。
6. `N1-0-05` 已冻结独立 `miniapp-release-v1`、UI-only/Service、MessageChannel、
   可选 Files/Private DB、additive Migration、Ready/Active/Previous、
   Publish/Rollback、Share/Backup/Delete 合同。Bridge wire 只含 call 与 payload，
   MiniApp/Release/epoch 由 Host 绑定，不接受 UI 自报身份。
7. `N1-1-01` 已新增 `nomifun-js-runtime`：
   - 仅发现用户路径、已保存路径、当前 PATH 与 managed root；
   - Windows x64 managed Node 必须由用户确认，版本来自 Node 官方 LTS index，
     archive 按官方 `SHASUMS256.txt` 校验后 staging/原子发布；
   - probe 校验 Node identity/version/target/execPath/SHA-256，global candidate 未完成
     exact validation 前不得提交；
   - 本机真实 PATH Node probe 已通过。
8. 当前验证：`nomifun-agent-contracts` 79 tests、`nomifun-agent-kernel` 23 tests、
   Control Plane 16 tests、Agent Platform 2 tests、JavaScript Runtime 12 tests、
   App tests compile、generator `write/check`、Gate self-test 均通过。
   `cargo fmt --all --check` 在 Windows 命中文件名过长限制，改用受影响 crate 的定向
   `cargo fmt --check`；该平台限制不阻断已通过的编译与定向格式验证。
9. clean detached worktree 上的正式 `contract/combined` Gate 已 PASS：
   source `6a6ff176974a861da064de3d95a8c953a55ff655`，cohort digest
   `546f5fa2acfed95d58006bc27314decf4d6df1deb34dd22495f3046335161539`，
   evidence 位于 `build.noindex/agent-capability-v2/6a6ff1769/n1-contract/`。
10. `N1-1-02` 已交付 lazy shared Extension Host：stdio NDJSON 私有 IPC、exact Hello、
    demand-load、Mount handle 唯一、普通 rejection 隔离、request cancel、原子
    quiescent admission fence、watchdog、整进程树回收、late response generation fence 与
    next-demand restart，10 项真实 Node 测试通过。
11. `N1-2-00` 已交付 immutable Artifact Store：directory/zip containment、严格 JSON、
    Windows case/NFC collision、symlink/special file、流式大小/digest、cancellation cleanup、
    staging/atomic CAS publish 与 published inventory/tamper 校验，9 项测试通过。
12. `N1-2-01` 已由提交 `6debcb628` 交付 migration 067/068：Plugin Artifact/Project/
    Candidate/TestReceipt/Mount/current/previous/KV/Credential/Operation 及全局 Runtime
    selection。Repository、schema、migration/restart 合计 64 项定向测试通过；同
    package version 的不同 digest 可并存，delete-data 保留 Project/Ready/Test 资产。
13. Plugin/MiniApp 产品 DTO 已冻结 Runtime、Library、Workshop、Candidate/Release、
    Config/Credential reference、Operation 与生命周期动作；不暴露 secret、Host
    generation、Bridge、localhost、旧 Extension 或 Conversation/Guid MiniApp 字段。

## 2026-09-07 实施记录

1. `N1-2-02` 已由提交 `a275ddd97`、`a16cfbeff` 关闭：owner mutation coordinator、
    migration 069、Config/Credential/KV exact CAS、stable `dataDir` 和 Runtime state
    查询均已进入真实 SQLite repository。Plugin repository 12、ID/schema 20、lifecycle
    31 项定向测试通过。
2. `N1-1-03` 已完成普通 Plugin Capability 的唯一 Kernel 执行主链：
    - `PluginRegistration` 按 Capability kind 登记 Tool、Context、Resource typed export；
    - Shared Host 公开 Context contribute、Resource acquire/release，资源句柄绑定 Host
      generation，旧 generation 的延迟 release 不会误伤新进程；
    - Agent Context 组装可按 materialized capability 在普通 Mount 与 canonical Role
      Provider 路径间精确分发；
    - Snapshot 的完整 `ResolvedCapability` 与 Artifact digest 进入 Adapter 校验和资源
      handle cache key，Compatible Replace 不复用旧 Artifact 资源；
    - `plugin-package-v1` 明确拒绝 Role Provider 和 Package-authored Plugin Service，
      不建立 JS 专用 Role schema 或第二 Registry。
3. `N1-2-03` 已进入 application-service 实施：`nomifun-plugin-service` 已接真实
    `SqlitePluginN1Repository`、Artifact Store、owner mutation、Host commit fence、
    Candidate/Apply/Restore/Uninstall/Delete-data 与 Operation CAS；application service
    不提供绕过 Kernel 的直接 invoke。12 项 application tests 和 6 项真实 SQLite
    adapter tests 已通过；App composition、Kernel publication/invalidation、
    Build/Test cancellation 与完整 E2E 尚未完成，因此不关闭。
4. `nomifun-miniapp-platform` 已形成不读取旧 `miniapps` 的 M1 domain/application
    foundation，并以 5 项内存合同测试固定 Product/Project、Ready/Active/Previous、
    Catalog 原子语义、生命周期、auto-publish authorization 和 resumable delete。
    该预研不含 migration 070+、SQLite adapter、Service Host、Bridge、UI 或旧链删除，
    不改变 `M1-0-01` 的 blocked 状态，也不计入 M1 完成度。
5. `N1-2-03` 已接入 NomiCore 当前组合根：
    - `PluginApplicationService` 通过真实 SQLite/Artifact Store/Shared Host fence 与
      Kernel Registry publisher 组合；
    - 启动恢复 enabled ManagedLocal Mount，按 exact Artifact/Config/Credential/dataDir
      重建 registration；卸载/停用会从唯一 Registry 撤销；
    - `/api/plugins`、`/api/plugin-projects/*`、`/api/plugin-mounts/*` 和
      `/api/plugin-operations/*` 已加入 owner + local-trust 管线；
    - compiler 使用动态 `KernelRegistry` provider，不缓存旧 materialized registry；
      在 N1-3 consumer adapter 完成前，Plugin capability 在 Agent Catalog 中明确
      `CAPABILITY_UNAVAILABLE`，不伪造可执行。
6. `N1-4-01` 已开始实施：新增 `nomifun-js-authoring`，复用平台统一
    `DigestHex/UserId/PluginProjectId`，完成 owner/project Source Store、JS/TS
    scaffold、canonical source snapshot、exact dependency request/lock 合同及
    staging/cancel cleanup。Source Store 现在持有 canonical `dependency-lock.json`，
    Project 创建会把真实 Source/lock digest 写入数据库；17 项 authoring tests 通过。
    npm resolver/
    cache、fixed packer、Build Host、DB/App 接线仍未完成。
7. 当前定向验证：JavaScript Host 13、JS Kernel Adapter 4、Plugin Service 16、
   JavaScript Authoring 16、MiniApp Platform 5、App Plugin publisher E2E 1 均通过；
   App check、contract generator `write/check` 与 Plugin N1 Gate self-test 通过。bundled
   Host 脚本按内容 digest 物化到受管数据目录，安装版不依赖编译机源码路径。
8. migration 070 已把 Plugin Project 的 `display_name/description` 从 Package ID 中
   物理拆出并为既有行一次性回填；Library/Workshop 返回真实产品元数据。Project 删除
   使用 owner/revision/build generation/Ready Candidate identity+digest 精确 CAS，
   只删除 Project/Source/lock/Ready，保留已安装 Mount 与运行数据；提交后的 Source
   清理失败明确返回 `PLUGIN_RECONCILE_REQUIRED`。
9. `N1-U-01` 已进入实施：主导航 `/plugins`、Plugin Library、Project Workshop、
   创建、预构建导入、Build/Test/Apply、Operation cancel、Project 删除和 Mount
   生命周期动作已接真实 HTTP bridge；MCP 页面不再重复承载 Plugin 设置。Plugin UI
   定向 14 tests、i18n Gate 与 production UI build 通过；Node Runtime Manager、
   Config/Credential 编辑、真实 Build/Test 可用态和 Desktop accessibility/视觉走查
   仍未完成，因此不得关闭。
10. Candidate Test Host 已从 Shared Extension Host 物理分离：同一 Supervisor 基础设施
    按 `shared_extension | candidate_test` 严格绑定 Hello role、request envelope 和
    generation，但两者始终使用不同 Node 进程。NomiCore 的真实 Candidate Test executor
    为每次测试分配一次性 dataDir、不注入生产 Credential、加载并激活 exact Artifact，
    随后停止并证明测试进程树归零；有可调用 contribution 但尚无受管测试输入时记录
    `needs_test_input`，不伪造 `passed`。JavaScript Host 14、Adapter 4 和 App publisher
    E2E 均通过。
11. `N1-3-01` 已进入正式 Catalog 合流，`N1-3-03` 已关闭：
    - `KernelCatalogProvider` 直接把 Kernel 已物化的 ManagedLocal Catalog entry 交给
      Control Plane，保留 exact Mount、contribution、contract 与 Artifact provenance；
      Agent catalog、operation lock 和 consumer filtering 不再从 raw manifest 重建或猜测；
    - 普通 Plugin Tool 新增 source-neutral non-Agent operation context/handler，不创建
      AgentSession/Preset/Snapshot，也不复用 Role Provider operation 类型；
    - App E2E 证明同一 Plugin contribution 对 Agent 因 Nomi adapter 尚未完成而明确
      unavailable，但 Gateway exact lock 可实际调用成功；
    - JS adapter fixture 同时覆盖 Agent+Gateway 共享 Tool 与 UI-only Tool；UI-only
      contribution 不进入 Agent consumer，旧 Artifact lock 在 Host 前 fail closed。
      Kernel 23、Control Plane 16、JS adapter 4 与 App publisher E2E 均通过。

## W0：一期交接

| ID | 状态 | 目标 | 完成定义 | Evidence |
| --- | --- | --- | --- | --- |
| `W0-01` | closed | 关闭一期 Windows C8 | 候选 package/install/fresh/launch、真实 StepFun、Browser/Computer、Remote、cleanup、release lock 全部通过 | candidate `0bac72da4ebb62f6a0f183a1285065c88aa684a4`; Host `555a4560...`; NSIS `d0f22840...`; lock `54e39e87...` |

## N1-0：机器合同冻结

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-0-01` | closed | 集成 Owner；全仓只读扫描、06/本文 | repo dependency/product residual scan，冻结删除顺序和真实消费者 | `W0-01` | 生产 `/api/presets`=0；Extension/MiniApp 消费者与删除顺序已登记 |
| `N1-0-02` | closed | Contract lane；`nomifun-agent-contracts/src/plugin_*`、`contracts/plugin-n1/**`、`nomifun-api-types/src/plugin*` | 冻结 `plugin-package-v1`、Host IPC、Runtime fingerprint、Credential slot、Project/Candidate/TestReceipt/Apply/Share 合同 | `N1-0-01` | 62 contract tests；generator write/check；workspace check |
| `N1-0-03` | closed | Kernel lane；`nomifun-agent-kernel/src/{materialize,registry,compiler,plugin,error}.rs` | Snapshot/operation 精确锁定 mount/contribution/artifact，不依赖无关全局 generation | `N1-0-02` | Kernel 23、Control Plane 16、Agent Platform 2；unrelated publication 与 drift tests |
| `N1-0-04` | closed | Gate lane；`scripts/gate-plugin-n1.mjs`、validation result | 建立 Windows N1/M1 stage、required/optional cell 和最终 cohort 合同 | `N1-0-02` | Gate self-test/dry-run；无 source SHA pre-run input；pending check fail-closed |
| `N1-0-05` | closed | M1 Contract lane；`nomifun-agent-contracts/src/miniapp_m1.rs` | 冻结独立 `miniapp-release-v1`、Service/Bridge/Storage、Ready/Publish/Rollback/Share/Delete 合同 | `N1-0-02` | canonical manifest/envelope/schema；17 M1 contract tests；无 Plugin 顶层 Manifest复用 |

## N1-1：Node Foundation

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-1-01` | closed | Runtime lane；新 `nomifun-js-runtime/**` | Node PATH/手工/managed LTS probe、下载确认、fingerprint 与全局试切换 | `N1-0-02` | 12 tests；真实 PATH Node；official index/SHASUMS/zip containment；candidate validation |
| `N1-1-02` | closed | Runtime lane；新 `nomifun-js-host/**` | lazy shared Extension Host、独立 Candidate Test Host、private IPC/Hello、watchdog、whole-tree cleanup、late-result fence | `N1-1-01` | Host 14；role-isolated process、demand=0、crash/restart、child cleanup、quiescent fence、installed-path materialization |
| `N1-1-03` | closed | Kernel+Runtime 边界；`nomifun-js-kernel-adapter/**`、Kernel typed exports、Shared Host API | 普通 Plugin Tool/Context/Resource 的 Node proxy；N1 明确拒绝 Role Provider/Plugin Service | `N1-0-03`,`N1-1-02` | Host 13、Adapter 4、Kernel 23、Agent Platform 16；exact Artifact handle fence；无 Rust/Node 双 Registry |

## N1-2：Package 与数据生命周期

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-2-00` | closed | Artifact lane；`nomifun-plugin-platform/**` | directory/zip containment、canonical digest、immutable CAS staging/publish | `N1-0-02` | 9 tests；tamper/traversal/collision/cancel/idempotency |
| `N1-2-01` | closed | DB lane；migration 067/068/070、新 repository | Artifact/Project/Candidate/current/previous/Mount/Operation/retained data、Project display metadata 与 Runtime selection schema | `N1-0-02` | `6debcb628` + 当前 070；Plugin repository 13、ID schema 20；fresh/restart/direct-SQL guards |
| `N1-2-02` | closed | Plugin platform lane；migration 069、DB repository、owner mutation coordinator | Config、Credential slot binding、KV/CAS、stable `dataDir`、owner mutation lock | `N1-2-01`,`N1-1-02` | `a275ddd97`,`a16cfbeff`；repository 12 + ID/schema 20 + lifecycle 31 |
| `N1-2-03` | in-progress | Plugin application-service/App lane；`nomifun-plugin-service/**`、`nomifun-app/src/router/plugin_platform.rs` | staging/containment/digest/install/replace/restore/uninstall/delete-data/project-delete 与真实 App/Kernel 组合 | `N1-2-01`,`N1-2-02` | service 12 + SQLite adapter 6 + App publisher E2E 1；真实 metadata/TestReceipt/Operation、Candidate Test Host 已接；仍需真实 Build、Build cancellation、N1-3 consumer 与安装版验证 |

## N1-3：Catalog 与消费者

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-3-01` | in-progress | Catalog lane | ManagedLocal Package/Mount materialization、五类 Contribution、provenance/availability | `N1-0-03`,`N1-1-03`,`N1-2-03` | 正式 ManagedLocal entry 与 Tool/Context/Resource provenance 已接；仍需 Skill、MCP-backed 完整物化与 duplicate/fault closing |
| `N1-3-02` | blocked | Nomi consumer lane | 用动态 action schema/Kernel invoke 替换 Nomi 硬编码 Capability→Tool 表 | `N1-3-01` | AgentPreset compile/invoke/impact |
| `N1-3-03` | closed | 非 Agent consumer lane | 一个共享 Capability 与一个 non-Agent-only reference contribution | `N1-3-01` | source-neutral operation handler；Agent+Gateway shared Tool、UI-only Tool、exact lock/Artifact drift 与 Agent filtering 均通过 |

## N1-4：Authoring 与 Self-Evolution

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-4-01` | in-progress | Authoring lane；`nomifun-js-authoring/**` | Source Store、JS/TS scaffold、pure-JS dependency exact lock、fixed packer/Build Host | `N1-0-02`,`N1-1-02`,`N1-2-01` | authoring 17；Source/host lock/DB create 接线通过；仍需 npm resolver/cache、packer、Build Host 与真实 cancellation |
| `N1-4-02` | blocked | Plugin project lane | single Ready、Candidate Test、impact、manual/compatible-idle Apply、Retry/Restore | `N1-2-03`,`N1-3-01`,`N1-4-01` | stale base/busy/breaking/no auto rollback |
| `N1-4-03` | blocked | SDK/CLI lane | Plugin SDK、Share Bundle/prebuilt import/export、本地 CLI | `N1-4-02` | same application service；secret-free bundle |

## N1-X：旧 Extension 删除与产品 UI

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-X-01` | closed | Skill lane；新 `nomifun-skill-library/**` | 把 `skill_service`、builtin skills、Skill market 从 `nomifun-extension` 抽为独立 owner | `W0-01` | `610b59007`；Skill Library 150 passed/2 ignored；Extension/消费者 checks passed |
| `N1-X-02` | blocked | demolition lane；`nomifun-extension/**` 及消费者 | 删除旧 Extension loader/registry/hub/hot reload/permissions/settings/webui/agent/theme 路径 | `N1-X-01`,`N1-3-03`,`N1-4-03` | `/api/extensions/*`、Hub、`nomi-extension.json` 生产可达性为 0 |
| `N1-U-01` | in-progress | UI lane；新 `pages/plugins/**`、Runtime Manager | Plugin Library/Workshop/配置/诊断、Node Runtime Manager；MCP 页面只保留 MCP | `N1-2-03`,`N1-4-02` | Library/Workshop/authoring actions、14 tests、i18n、production build 已通过；仍需 Runtime Manager、Config/Credential 编辑及 Desktop product/a11y 走查 |
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

`nomifun-miniapp-platform` 当前只作为上述 M1 工作的提前 domain foundation 保存；在
`N1-V-01` 关闭并交付 migration/SQLite/Host/Bridge 前，不领取任何 M1 `in-progress`
状态，不接生产路由，也不替代旧 MiniApp 主链。

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
