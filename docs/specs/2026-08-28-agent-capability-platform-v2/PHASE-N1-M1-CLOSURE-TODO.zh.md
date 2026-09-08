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
> 2026-09-08 已同步远端 AgentPreset/会话模型选择修正；M1-0-02-A、M1-0-02-B、
> M1-1-01 与 M1-1-02 已完成实现和 Windows 定向回归。M1-1-01 已交付 dedicated
> Service Host、真实 Node process adapter、on-demand/continuous、candidate Runtime
> 验证和 Service Bridge；M1-1-02 已交付 owner-scoped Files、Host-managed Private
> SQLite、authorizer、参数化 query/execute/batch、additive Migration ledger 与
> Publish migration fence。M1-2 的 Trash/Restore/Permanent Delete 与启动恢复子切片
> 已由 `86afa7af6` 完成；下一步进入 Service Test 和 Share/Backup Import-as-new。
> Windows Candidate、
> NSIS、产品验收和 macOS/Linux 外部验证仍未关闭。

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
| 已关闭 | 20 | 当前表内已关闭的 W0/N1/M1 项，包含 `M1-0-01`、`M1-0-02-A`、`M1-0-02-B`、`M1-1-01`、`M1-1-02` |
| 正在实施 | 5 | `N1-1-01`、`N1-2-03`、`N1-4-01`、`N1-U-01`、M1-2 生命周期 lane |
| 已解锁待领取 | 0 | 当前 Windows 主线无未领取的前置切片 |
| 依赖阻塞 | 8 | 其余 N1/M1 Windows 项与最终合流 |
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
4. `nomifun-miniapp-platform` 已形成不读取旧 `miniapps` 的 M1
    domain/application/runtime/data foundation，并以 15 项内存合同测试固定：
    - Product/Project、Ready/Active/Previous、Catalog 原子语义、生命周期、
      auto-publish authorization 和 resumable delete；
    - Host-owned MessageChannel port 的 exact Active pointer/Release/epoch/Surface fence，
      UI-only 只允许 Host KV，旧 port/旧 generation 迟到结果 fail closed；
    - dedicated Service Host 的 on-demand/continuous、全局动态容量、有限 crash backoff、
      用户 Retry、idle reap 和单 App 故障隔离；
    - owner-scoped KV checked revision/CAS、Files handle、Private DB 窄 SQL、
      additive migration ledger，以及“取消只在提交前生效”的副作用边界。
    该提前 foundation 不含 migration 071+、真实 SQLite/Node process adapter、生产
    composition/routes、MiniApp UI 或旧链删除，不改变任何 M1 条目的 blocked 状态，
    也不计入 M1 功能完成度。
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
   JavaScript Authoring 16、MiniApp Platform 15、App Plugin publisher E2E 1 均通过；
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
12. Plugin Library 已接入真实 Config/Credential 编辑：
    - 按当前 exact JSON Schema 渲染 string/number/integer/boolean/enum，unsupported、
      nested、dynamic 或 read-only schema 明确只读并阻止全量替换；
    - `password/writeOnly/secret` 标记和疑似 secret 配置键不进入 draft、DOM 或请求，
      必须由 Package 改为 owner-scoped Credential slot；
    - Credential 按 §4.1B 只支持 `credential_id` reference 的 keep/bind/unbind，不新增
      Credential 枚举或 secret 回显；
    - 提交复用 Mount/current/config/schema/credential 全部 exact CAS，失败保留表单。
      Plugin UI 定向 28 tests、i18n、icons 与 production build 通过；真实 Desktop
      accessibility/视觉走查和 Runtime Manager 尚未完成，因此 `N1-U-01` 保持
      `in-progress`。
13. `N1-3-01` 已关闭：
    - Kernel 的 Tool、Context、Resource、Skill 和 MCP 五类物化都保留正式 owner、
      contribution、contract、Mount 与 Package Artifact facts；
    - ManagedLocal MCP-backed Capability 同时保留 MCP binding identity 和真实 Mount，
      不再用 mapping schema digest 冒充 Package Artifact，也不复制第二个 MCP
      contribution；
    - ManagedLocal Skill 使用 canonical `skill:<skill_id>` contribution identity，
      exact lock 进入 AgentPreset Revision，Mount/contract 漂移 fail closed；
    - Control Plane Catalog、Compiler 和 impact 直接消费 Kernel materialization，删除
      package/source-kind 推测、合成 Mount 和 synthetic MCP impact；
    - duplicate Skill/MCP、missing target、cross-package mapping 在 Registry swap 前失败，
      旧 generation/digest 原样保留。Kernel 26、Control Plane 17、Agent Platform 23、
      JS Kernel Adapter 4 项测试通过。
14. JavaScript Authoring 与真实 Plugin Build 主链已完成一轮可运行收口：
    - `nomifun-js-authoring` 已交付 canonical Source Manifest、JS/TS scaffold、Source
      snapshot、exact dependency lock、content-addressed pure-JS npm cache/resolver、
      static local/npm module graph bundler、Node 24 Build Host、resources copy、immutable
      `plugin-package-v1` packer 和 cancellation/staging cleanup；
    - fixed bundler 允许受控 `node:*` public API（拒绝 `node:module`）、拒绝 dynamic
      import/require、native addon、lifecycle script、未锁定依赖、路径逃逸、环和未知语法；
      diamond graph 复用 exact exports，多声明 export 等未支持语法 fail closed；
    - App 组合根使用同一 committed Node fingerprint，将 `authoring`、`npm-cache` 和
      `artifact-store` 放在互不包含的受管根；Build 输出必须再次经过 Artifact Store
      containment/digest admission 后才进入数据库；
    - `FsPluginBuildExecutor` 与 operation cancellation 共用 exact operation flag；
      cancel 等待 Node process/staging 清理，跨过取消边界后返回 conflict，不把成功
      Candidate 事后改写为 canceled；
    - Build 的 base target 与 contract diff 由 application service 从 exact linked
      Mount/current 和 canonical Manifest 计算，Builder 无权自报 compatibility。
      Authoring 27、Plugin Service 22、真实 Node Build/Artifact E2E 2、App crate check
      均通过。
    生产 npm registry client、Project dependency lock 更新、Chat Dev Source 编辑、
    Share/CLI 和 installed-app Build→Test→Apply 仍未完成，因此 `N1-4-01`、
    `N1-2-03` 均保持 `in-progress`。
15. `N1-3-02` 已关闭，普通 ManagedLocal Plugin Tool 不再依赖 Nomi 的硬编码
    Capability→Tool 表：
    - `plugin-package-v1` Artifact 正式携带 exact、content-addressed schema registry；
      contribution 引用必须使用 `schema://...#<sha256>`，missing/extra/tamper 均在
      admission 前 fail closed；
    - Nomi Runtime 在 single-flight factory 内按持久化 Conversation→Binding→Revision→
      Snapshot 链解析 exact Plugin Tool Session，动态 provider name、input schema、
      effect category 与 deferred ToolSearch 均由冻结 Snapshot 派生；
    - 每次调用携带 engine-owned operation/idempotency/correlation identity，并进入唯一
      Kernel invoke；当前 Mount/Artifact/schema 与 Snapshot 不一致时在 Host dispatch 前
      fail closed，不回退到 latest Catalog；
    - App-owned schema resolver 只从冻结 Artifact digest 读取 schema，UI-only、non-Agent、
      Hidden action 不进入 Nomi Tool Registry；native Nomi tools 保持原路径。
      Agent Contracts 80、动态 Plugin Tool 4、Runtime provider single-flight 1、
      JS Kernel Adapter 4 和 App publisher E2E 1 项验证通过。
16. Runtime Manager 后端 foundation 已扩展，但尚未形成生产切换闭环：
    - migration 071 与 SQLite adapter 持久化 selected/pending Runtime 的 absolute
      executable path，并保留 revision CAS、validation 与一次性非推荐确认；
    - `/api/javascript-runtime/status|probe|download|switch/*` 已进入 installation owner +
      local-trust 路由，支持 PATH/手工/Managed probe、official offer digest、异步下载状态、
      exact candidate reprobe 与 typed error；
    - Candidate Foundation Host Hello/stop/process-tree-zero 已可验证。当前 Plugin Host、
      Candidate Test、Build 与 MiniApp Service 尚未统一接入 committed Runtime authority，
      production validator 因此明确返回 `JAVASCRIPT_RUNTIME_NOT_COVERED`，不会伪造切换成功。
      JavaScript Runtime 16、Runtime SQLite 2、App Runtime route 3 项测试通过。
17. 2026-09-07 重新打开 `N1-1-01`：只完成 probe/download/CAS 不足以满足
    §4.3C 的全局 Runtime authority。当前新增并已接入组合根：
    - `RuntimeAuthority` 是唯一 committed Runtime 来源；Plugin Shared Host、Candidate
      Test Host 和 Build Host 均按 exact fingerprint/path 获取 read lease，不再各自扫描
      PATH 并永久捕获 Node；
    - migration 071 对 068 旧 selection 做 fail-closed reselection；启动会 exact reprobe
      已保存 executable，替换/删除时清空 selected 并记录 typed stale error；
    - `CoordinatedRuntimeSwitch` 持有全局 write fence，先 durable pending、drain/stop、
      Foundation/Plugin Mount validation，再 prepare、selection CAS、finalize；局部失败
      保留 pending 供用户裁决，恢复失败不释放 fence；HTTP 请求取消不会取消 detached
      coordinator task；
    - Runtime-bound Kernel Resource 在切换前统一 release，Plugin enable 在无 committed
      Runtime 时于 DB mutation 前拒绝。当前 MiniApp Service participant 明确返回
      `NotCovered`，因此生产切换仍不会伪造全通过。
      定向证据：Runtime 21、Runtime SQLite 3、App Runtime route 3、Plugin application
      15、App Plugin E2E 1；受影响 crate `cargo check` 与 fmt 通过。
18. `N1-2-03` 的组合根已改为 Runtime-bound：
    - Shared Extension Host、Candidate Test Host、Build Host 都只消费同一个 committed
      Runtime authority；Host port 以 instance+generation fence 拒绝旧 Resource release；
    - Candidate Runtime 验证会加载全部 enabled ManagedLocal Mount 的 exact Artifact、
      临时 dataDir（不注入生产 Credential）并证明候选 Host 停止；固定 Build Host 同时
      执行最小 JS 编译验证，普通业务 Tool 不在验证阶段执行；
    - 无 Runtime 时 Package 仍可安装/保留，但 Enable 在 owner mutation 前返回
      `PLUGIN_RUNTIME_UNAVAILABLE`，不会出现 DB `enabled=true` 与实际不可执行的分裂。
19. Runtime Manager Desktop UI 已替换旧本机 Agent 检测页面：
    - `/settings/execution-engines` 现在只承载 Runtime Manager，提供 status/probe、
      手工 Node 路径选择、官方 LTS 确认下载、非推荐版本确认和局部失败裁决；
    - `28ee78526` 的 bridge/model/交互测试通过，四组定向 UI 测试共 11 项通过；
      全量 UI typecheck 仍受仓库既有 Arco/React 类型错误阻断，尚未作为 N1-U-01
      关闭证据；Desktop accessibility/视觉验收也未完成。
20. M1-0-01 已开始 clean-start 数据根实施，但不改变 M1 条目当前 blocked 状态：
    - migration 072 新增 owner-scoped `miniapp_library_state`、Product、Project、
      immutable Artifact/Release 与 Credential reference 表，完全不读取或复制旧
      `miniapps`，并遵守 Fresh-v4 的无物理 FK/trigger logical-reference 合同；
    - `nomifun-db` 已有对应 row model 与 owner-scoped repository，支持 Library revision、
      Product+Project create、Project source CAS、Ready Release exact lineage/CAS 和
      Ready/Active/Previous pointer CAS；
    - DB contract 20、M1 schema 2、M1 repository 3 项定向测试通过。生产 App routes、
      MiniApp Service/Bridge/Storage adapter、旧链删除和 UI 尚未接入，不能计入 M1 完成度。

## 2026-09-07 M1-0-02-A 实施记录

1. 远端分支已核对与当前 `HEAD=d2180373a` 对齐；本轮不处理未跟踪的 `.githooks/`，
   也不重写历史提交。
2. `M1-0-01` 进入生产实现阶段：继续沿用 migration 072 与 owner-scoped
   `nomifun-db` repository，生产代码只读写新的 Product/Project/Artifact/Release
   数据根，不读取、迁移、双写或 alias 旧 `miniapps`。
3. 新增实施切片 `M1-0-02-A：UI-only Source → Build → Ready`，写集固定为：
   - MiniApp 专用 owner/project Source Store，保存 `index.html`、`ui/**` 与 canonical
     source snapshot；不复用 Plugin Source Store，也不伪造 Service Source；
   - 固定 `miniapp-release-v1` 的 UI-only Build application service、Build Operation
     start/finish/cancel 与 staging cleanup；
   - immutable Release Store，保存 manifest、文件 bytes、逐文件 digest 和完整 Release
     digest，并在写入 Ready 前再次执行 containment/digest admission；
   - Project source CAS、Ready Release lineage/CAS 与 Build generation 串行绑定；
   - owner + local-trust 保护的 Build 路由及 Library/Workshop 的最小 Build 交互。
4. `M1-0-02-A` 明确不包含 Service Host、MessageChannel Bridge、Files/Private SQLite、
   Publish、Rollback、Share/Import、旧 MiniApp 链删除或跨平台验证。UI-only Build 的
   成功和失败都必须证明 Node process 为 0；失败不得改变既有 Ready、Active 或 Previous。
5. 该切片完成后才进入 `M1-0-02-B：Ready → Manual Publish → Surface → Rollback`；
   在 `RC-WIN-01` 关闭前继续只做 Windows Desktop x64，不运行手机视口，也不交接
   macOS/Linux 原生验证。

## 2026-09-08 M1-0-02-A 收口与远端设计同步

1. 已快进同步远端 `rf/agent-capability-platform-v2` 的 AgentPreset/会话模型选择修正
   （当前远端提交包含 official Agent 选择、会话级模型覆盖、模型 alias 编辑及对应
   API/SQLite/UI 测试）；这些改动已通过本轮定向回归，不与 MiniApp 数据根混写。
2. 远端新增 `076_agent_session_model_configurations.sql` 后，本地 MiniApp Build
   lineage migration 顺延为 `077_miniapp_build_operation_lineage.sql`，数据库生命周期
   的正式 head 常量同步为 `77`；迁移链、ID/schema 契约和升级回归均已重新通过。
3. `M1-0-02-A` 已完成实现定义并关闭：
   - 新增独立 MiniApp owner/project Source Store 与 immutable Release Store；
   - UI-only `Source → Build → Ready` application service、Build Operation start/
     finish/failure/cancel、staging cleanup 和单 MiniApp single-flight 已接入真实
     SQLite repository；
   - Artifact/Release/Ready pointer/library revision/Operation 成功状态在一个 SQLite
     transaction 内提交；任何 CAS、digest、lineage 或 timestamp 失败均回滚；
   - App 已提供 owner/local-trust 保护的 create/workshop/build/cancel 路由，Desktop
     Workshop 只暴露 Source、Build、Ready、Cancel、Refresh，不伪造 Publish/Surface/
     Service 操作；
   - UI-only Build 固定不启动 Node，Source/Release Store 对路径 containment、Windows
     collision、特殊文件、digest tamper 和 owner/project 越界 fail closed。
4. 本轮定向证据：
   - `nomifun-db`：ID/schema 20、MiniApp schema 6、MiniApp repository 8、数据库生命周期
     31；
   - `nomifun-miniapp-platform`：Source/Release Store 3、M1 application 3；
   - `nomifun-app`：MiniApp route/application 3；
   - API types 535、Agent control plane 22、Agent session model route 1；
   - Desktop MiniApp UI 9、Agent/模型选择 UI 32、i18n check 与 UI production build
     通过；
   - `cargo check -p nomifun-db -p nomifun-miniapp-platform -p nomifun-app` 通过。
5. `cargo fmt --all -- --check` 在 Windows 仍命中仓库既有文件名长度限制；受影响 crate
   的定向格式检查与 `git diff --check` 通过。未跟踪 `.githooks/` 和测试运行时目录
   不进入提交。
6. 当前只领取 `M1-0-02-B`：Ready → Manual Publish → Surface → Rollback。该切片仍不
   包含 dedicated Service Host、真实 MessageChannel adapter、Files/Private SQLite、
   Share/Import、永久删除或 macOS/Linux 验证；这些项目继续按依赖保持 blocked。

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
| `N1-1-01` | in-progress | Runtime lane；`nomifun-js-runtime/**`、App composition、Plugin Runtime boundary | Node probe/download、唯一 committed Runtime authority、全局 admission fence、drain/stop/validation/commit-or-restore | `N1-0-02` | Runtime 21 + SQLite 3 + App route 3；authority/lease、071 reselection、pending recovery、Runtime-bound Host/Build/Test 与 fixed Build Foundation 已接；仍需真实 installed switch/fault/restart Gate |
| `N1-1-02` | closed | Runtime lane；新 `nomifun-js-host/**` | lazy shared Extension Host、独立 Candidate Test Host、private IPC/Hello、watchdog、whole-tree cleanup、late-result fence | `N1-1-01` | Host 14；role-isolated process、demand=0、crash/restart、child cleanup、quiescent fence、installed-path materialization |
| `N1-1-03` | closed | Kernel+Runtime 边界；`nomifun-js-kernel-adapter/**`、Kernel typed exports、Shared Host API | 普通 Plugin Tool/Context/Resource 的 Node proxy；N1 明确拒绝 Role Provider/Plugin Service | `N1-0-03`,`N1-1-02` | Host 13、Adapter 4、Kernel 23、Agent Platform 16；exact Artifact handle fence；无 Rust/Node 双 Registry |

## N1-2：Package 与数据生命周期

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-2-00` | closed | Artifact lane；`nomifun-plugin-platform/**` | directory/zip containment、canonical digest、immutable CAS staging/publish | `N1-0-02` | 9 tests；tamper/traversal/collision/cancel/idempotency |
| `N1-2-01` | closed | DB lane；migration 067/068/070/071、新 repository | Artifact/Project/Candidate/current/previous/Mount/Operation/retained data、Project display metadata 与 Runtime selection schema/path CAS | `N1-0-02` | `6debcb628` + 070/071；Plugin repository 13、Runtime selection 2、ID schema 20；fresh/restart/direct-SQL guards |
| `N1-2-02` | closed | Plugin platform lane；migration 069、DB repository、owner mutation coordinator | Config、Credential slot binding、KV/CAS、stable `dataDir`、owner mutation lock | `N1-2-01`,`N1-1-02` | `a275ddd97`,`a16cfbeff`；repository 12 + ID/schema 20 + lifecycle 31 |
| `N1-2-03` | in-progress | Plugin application-service/App lane；`nomifun-plugin-service/**`、`nomifun-app/src/router/plugin_platform.rs`、Runtime-bound Host | staging/containment/digest/install/replace/restore/uninstall/delete-data/project-delete 与真实 App/Kernel/Runtime 组合 | `N1-2-01`,`N1-2-02` | service 15 + App publisher E2E 1；Runtime-bound Shared/Candidate/Build Host、enable runtime gate、metadata/TestReceipt/Operation 已接；仍需安装版闭环、完整 Candidate/Apply/Restore 与 fault Gate |

## N1-3：Catalog 与消费者

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-3-01` | closed | Catalog lane | ManagedLocal Package/Mount materialization、五类 Contribution、provenance/availability | `N1-0-03`,`N1-1-03`,`N1-2-03` | 五类 exact materialization；ManagedLocal Skill/MCP binding+Mount+Artifact；duplicate/fault atomicity；Kernel 26、Control Plane 17、Platform 23、JS Adapter 4 |
| `N1-3-02` | closed | Nomi consumer lane | 用动态 action schema/Kernel invoke 替换 Nomi 硬编码 Capability→Tool 表 | `N1-3-01` | content-addressed schema registry；Snapshot-bound provider/session；deferred ToolSearch；Kernel invoke/stale Artifact fail-closed；4 consumer + 1 single-flight + App schema E2E |
| `N1-3-03` | closed | 非 Agent consumer lane | 一个共享 Capability 与一个 non-Agent-only reference contribution | `N1-3-01` | source-neutral operation handler；Agent+Gateway shared Tool、UI-only Tool、exact lock/Artifact drift 与 Agent filtering 均通过 |

## N1-4：Authoring 与 Self-Evolution

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-4-01` | in-progress | Authoring lane；`nomifun-js-authoring/**` | Source Store、JS/TS scaffold、pure-JS dependency exact lock、fixed packer/Build Host | `N1-0-02`,`N1-1-02`,`N1-2-01` | authoring 27 + real Build Executor E2E 2；resolver/cache/static local+npm bundler/Node Host/Artifact admission/cancel 已接；仍需 production registry/lock mutation、MiniApp profile 和 Chat Dev Source edit |
| `N1-4-02` | blocked | Plugin project lane | single Ready、Candidate Test、impact、manual/compatible-idle Apply、Retry/Restore | `N1-2-03`,`N1-3-01`,`N1-4-01` | stale base/busy/breaking/no auto rollback |
| `N1-4-03` | blocked | SDK/CLI lane | Plugin SDK、Share Bundle/prebuilt import/export、本地 CLI | `N1-4-02` | same application service；secret-free bundle |

## N1-X：旧 Extension 删除与产品 UI

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-X-01` | closed | Skill lane；新 `nomifun-skill-library/**` | 把 `skill_service`、builtin skills、Skill market 从 `nomifun-extension` 抽为独立 owner | `W0-01` | `610b59007`；Skill Library 150 passed/2 ignored；Extension/消费者 checks passed |
| `N1-X-02` | blocked | demolition lane；`nomifun-extension/**` 及消费者 | 删除旧 Extension loader/registry/hub/hot reload/permissions/settings/webui/agent/theme 路径 | `N1-X-01`,`N1-3-03`,`N1-4-03` | `/api/extensions/*`、Hub、`nomi-extension.json` 生产可达性为 0 |
| `N1-U-01` | in-progress | UI lane；新 `pages/plugins/**`、`pages/settings/RuntimeManager/**` | Plugin Library/Workshop/配置/诊断、Node Runtime Manager；MCP 页面只保留 MCP | `N1-2-03`,`N1-4-02` | Plugin/Runtime bridge+model/interaction 39 targeted tests、i18n/icons/production build 已通过；全量 typecheck 有既有基线错误，仍需 Desktop product/a11y/视觉走查 |
| `N1-V-01` | blocked | 集成 Owner | Windows N1 contract/integration/fault/product/NSIS candidate | 所有 N1 项 | authored JS/TS → Test → Apply → invoke → Restore |

## M1：Full-stack MiniApp

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `M1-0-01` | closed | MiniApp DB/domain lane；`nomifun-db/migrations/072*`、`repository/miniapp_m1*` | 全新 Product/Project/Ready/Active/Previous 数据根，不读旧 `miniapps` | `N1-V-01` | migration 072、schema/repository、生产 owner 路由与 clean-start 数据根定向验证通过 |
| `M1-0-02-A` | closed | MiniApp release lane；`nomifun-miniapp-platform/**`、MiniApp application/App Build adapter | UI-only `miniapp-release-v1` 的 Source→Build→Ready、Build Operation、immutable Release Store | `M1-0-01`,`N1-4-01` | Source owner 隔离、成功/失败/cancel、文件 bytes/digest、Ready lineage/CAS、全程 Node process=0；DB 20+6+8+31、Store 3、Application 3、App route 3、UI 9 |
| `M1-0-02-B` | closed | MiniApp release lane；Publish/Catalog/Surface adapter 与 Desktop Workshop | Ready→Manual Publish→Surface→Rollback、纯 UI auto Publish、Host KV | `M1-0-02-A`,`M1-1-01` | UI-only Release/Catalog 原子切换、Active/Previous pointer CAS、Surface epoch fence、UI 定向验证通过 |
| `M1-1-01` | closed | Service/Bridge lane；`nomifun-miniapp-platform/src/{service_host,service_process,service_runtime}.rs` | 单 `main.mjs`、dedicated Host、on-demand/continuous、MessageChannel epoch fence | `M1-0-02`,`N1-1-02` | 真实 Node NDJSON 3、Service application 1、Runtime candidate 3、旧 generation/崩溃隔离/容量/backoff 通过 |
| `M1-1-02` | closed | Managed data lane；`managed_storage.rs`、Service IPC、M1 cutover | UI/Service KV、Files、Private SQLite、authorizer、参数化 SQL、additive migration ledger | `M1-1-01` | production SQLite Storage 1、真实 Node Storage IPC 3、authorizer/批量回滚/启动与取消边界通过 |
| `M1-2-01` | in-progress | Lifecycle lane；MiniApp lifecycle/application/data cleanup | Enable/Disable/Trash/Restore/Permanent Delete、Service Test、Share/Backup Import-as-new | `M1-1-02` | `86afa7af6` 已关闭 durable delete、物理清理、Retry/启动 Reconciler 和 Desktop 操作；仍需 transient Service Test receipt、Share/Backup Import-as-new |
| `M1-U-01` | blocked | UI lane；整体重写 `pages/miniApps/**` | Library/Workshop/Surface，删除 Guid/Conversation 旧 MiniApp 模式 | `M1-0-02`,`M1-1-01` | real Desktop workflow/build/a11y |
| `M1-V-01` | blocked | 集成 Owner | Windows M1 contract/integration/fault/product/NSIS candidate | 所有 M1 项 | UI-only + Service representative lifecycle |

`nomifun-miniapp-platform` 的内存实现仍保留作为合同测试，但不再承担生产事实。当前
生产组合已接入 dedicated Node Service Host、candidate Runtime 验证、Host-owned
Surface/Service Bridge、owner-scoped Files、Host-managed Private SQLite 和持久
Migration ledger。Node Service 的 Storage 请求使用同一私有 NDJSON 通道，数据库路径
不进入 Service SDK/HTTP/renderer wire；Publish 在旧 Host 停止后执行 pending additive
Migration，再启动目标 Host，失败时保留旧 Active 并重建旧 Service。M1-2 生命周期
删除子切片已完成；下一执行切片为 transient Service Test 与 receipt，随后进入
Share/Backup Import-as-new。当前仍不构成 Windows Candidate 或跨平台完成。

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

## 2026-09-08 M1-0-02-B 收口记录

1. M1-0-02-B 已按 clean-cut 交付，未恢复旧 MiniApp 路由、旧 ui_asset_id 身份或
   localhost Bridge：
   - migration 078 增加 owner-scoped auto Publish authorization 与 Catalog publication；
   - migration 079 将 Release 身份固定为 release_id，允许相同 Artifact digest 对应多个
     Release lineage，并保留 SQLite AUTOINCREMENT 高水位；
   - migration 080 持久化 Host-owned Surface session，只保存 capability digest，不保存
     明文 capability；migration 081 为 Host KV 增加 tombstone 与单调 key generation，阻断
     Delete → Recreate 的 revision ABA；
   - Publish、Rollback、Enable/Disable、Catalog projection 和 Surface session 使用事务内
     exact CAS；Config/Credential mutation 在 running Build 期间 fail closed；
   - Artifact/Release record 使用 typed contract、完整 digest/identity 校验和 canonical JSON；
     Artifact Store 提供 load_exact，独立检测 artifact_id 篡改；
   - Surface 签发改为 local-trust 保护的 POST /api/miniapps/{miniapp_id}/surface/open；
     iframe 必须先完成 challenge/nonce handshake 才转移一次性 MessagePort；Close 只有
     Host 成功确认后才清理 descriptor，失败保留可重试状态；
   - UI-only Build 自动注入固定 Host Bridge bootstrap，Source Store 原始 bytes 不变，
     auto Publish 比较使用同一确定性物化规则。
2. 定向 evidence：
   - nomifun-db：db lifecycle 31、ID/schema 20、MiniApp schema 11、repository 13；
   - nomifun-miniapp-platform：library 17、M1 build 6、Source/Release Store 3；
   - nomifun-app：MiniApp route 3、MiniApp E2E 2；
   - UI MiniApp/Bridge/model/http 定向测试 51；Agent contracts 84；受影响 crate
     cargo check 与定向 cargo fmt --check 通过。
3. 明确边界：本记录不关闭 M1-1-01、M1-1-02、M1-2-01、M1-V-01，也不构成
   macOS/Linux 或手机模式验证。M1-1-01 的下一步是把 Service Host、真实 process
   adapter、Service Bridge、Files/Private SQLite 生产接入；Windows 完成前不交接外部
   原生环境。

## 2026-09-08 M1-1-01 / M1-1-02 实现收口

1. `M1-1-01` 已完成 Windows 主机实现：
   - 一个 MiniApp 一个 dedicated Node Service Host，固定 `on_demand` /
     `continuous` 生命周期、全局容量、idle reap、有限 crash backoff 和用户 Retry；
   - Node process 使用私有 NDJSON IPC，Hello、release、runtime、module digest、
     host generation 和 service run key 精确绑定；EOF、crash、timeout、迟到响应和
     process tree cleanup 均 fail closed；
   - Surface Service Bridge 保持 Host-owned scope，旧 Active epoch / generation 的
     callback 和 port 不可继续调用；
   - Runtime switch participant 已从生产 `NotCovered` 改为对 enabled Service 逐项使用
     candidate Node 启动验证，返回精确 MiniApp identity。
2. `M1-1-02` 已完成生产接线：
   - `SqliteMiniAppManagedStorage` 使用 owner + MiniApp 路径边界，Files 目录逐级拒绝
     symlink/reparse/junction 越界；Private SQLite 路径不进入 renderer、HTTP 或 Service
     database API；
   - Service KV 复用 M1 `miniapp_kv` 的 owner-scoped revision/tombstone/CAS；
   - Private SQLite 只接受参数化单语句 `query` / `execute` 和最多 64 条 DML `batch`；
     authorizer 拒绝 Attach、Detach、任意 PRAGMA、DDL、trigger/view、Host metadata
     和非主数据库访问；batch 事务权限与 Host migration schema 权限分离；
   - Migration 使用 immutable ID + digest ledger，在一个 SQLite transaction 中只执行
     CREATE TABLE/INDEX 与 ADD COLUMN；ledger digest/schema epoch 在重启后重算并校验；
   - Publish 在旧 Service 停止后执行 pending Migration、重新解析 Storage descriptor、
     启动目标 Service，再进入 pointer/Catalog transaction；目标失败时恢复当前 Active，
     不执行反向 Migration；
   - Node Service `context.storage` 通过同一私有 IPC 提供 KV、Files `filesDir` 和隐藏
     Private DB API；Storage callback 异步化并携带 parent invocation cancellation。
3. 定向证据：
   - `cargo check --locked -p nomifun-app -p nomifun-miniapp-platform -p nomifun-db`；
   - `cargo test --locked -p nomifun-miniapp-platform --lib --test service_process
     --test service_application --test managed_storage --test service_storage_ipc`；
   - 真实 Node Service process 3、生产 SQLite Storage 1、Node Storage IPC 3、Runtime
     candidate validation 3、App runtime participant 1 均通过；
   - `cargo fmt -p nomifun-miniapp-platform -- --check`、`cargo fmt -p nomifun-app
     -- --check`、`git diff --check` 通过。
   - 代码 checkpoint：`2520904e9`（生产 Storage 初接入）、
     `da2ce11a8`（Storage lifecycle/IPC hardening）。
4. 当前边界：本收口不关闭 `M1-2-01` 的 Trash/Restore/Permanent Delete、Share/
   Backup Import-as-new、Service Test 临时 namespace/receipt、Windows NSIS Candidate
   或 macOS/Linux 原生验证；这些继续按依赖推进。

## 2026-09-08 M1-2 生命周期删除子切片收口

1. `86afa7af6` 已完成全新 M1 数据根上的产品生命周期闭环：
   - migration 082 新增 owner-scoped `miniapp_deletion_intents`；Trash、Restore、
     begin/fail/restart/finalize Permanent Delete 均使用单 SQLite transaction 和 exact CAS；
   - Trash 原子撤销 Catalog/Surface，Service 使用 exact MiniApp identity 停止；Restore
     固定返回 `disabled`，不会恢复执行、Surface 或 Catalog；
   - Permanent Delete 写入不可取消的 `miniapp_permanent_delete` Operation 后，从头幂等
     清理 Service、Files、Private SQLite、Source、Release 和 M1 owner rows；失败保留
     intent/Operation，用户 Retry 与启动 Reconciler 均可继续；
   - finalize 只在物理清理成功后删除 Product/Project/Release/Config/Credential/KV/
     Surface/Catalog rows，并保留成功/失败 Operation 历史。
2. Windows 文件系统边界已加固：
   - Source、Release、Files 和 Private SQLite purge 逐级验证受管父链、canonical
     containment，并在删除前递归拒绝 symlink、junction、reparse point 和 special file；
   - Private SQLite `-wal/-shm` 使用原生 `OsString` 拼接，不经有损 `display()`；
   - restart recovery 不依赖内存 storage registration，重复清理已不存在的目标成功。
3. Desktop Workshop 已接入移入回收站、恢复、永久删除和失败重试：
   - 所有动作使用产品确认对话框和 exact request，删除中限制其他写操作并轮询 durable
     Operation；成功后返回 Library，失败后重载 `deleting + failed` 状态并显示 Retry；
   - 中英文文案、i18n key、model 和 HTTP bridge 均已同步；普通 active lifecycle 控件
     不会在 trashed/deleting 状态继续显示。
4. 定向证据：
   - `cargo test --locked -p nomifun-db --test miniapp_m1_repository
     --test miniapp_m1_schema`：18 + 12 passed；
   - `cargo test --locked -p nomifun-miniapp-platform --test m1_application
     --test managed_storage --test miniapp_source_release_store
     --test service_storage_ipc`：5 + 2 + 7 + 3 passed；
   - `cargo test --locked -p nomifun-app --lib router::miniapp_m1::tests
     --no-default-features -- --test-threads=1`：6 passed；
   - MiniApp UI/wire 定向测试 26 passed；i18n parity/generation、受影响 crate check、
     定向 rustfmt 和 `git diff --check` 通过。
5. 当前边界：`M1-2-01` 仍为 `in-progress`。本子切片没有实现 Service Test 的 transient
   KV/DB/files namespace 与 Host-issued receipt，也没有实现无用户数据 Share Bundle、
   source-less prebuilt Import 或 disabled Whole-App Backup Import-as-new；`M1-U-01`、
   `M1-V-01`、Windows NSIS 和 macOS/Linux 原生验证均未关闭。
