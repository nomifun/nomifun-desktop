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
> 已由 `86afa7af6` 完成；Service Test transient namespace/receipt 已由 `708ef83b7`
> 完成。Share Bundle Export/Import、source-less prebuilt Import-as-new、Desktop Transfer
> UI 和 disabled Whole-App Backup Export/Import-as-new 已完成实现与 Windows 定向回归。
> Whole-App Backup 还覆盖 Service Files、Private SQLite、Migration ledger、Catalog
> identity digest 和 owner operation 互斥。
> Plugin Authoring 的 production npm registry adapter 已形成并通过真实 public npm
> smoke；dependency request/lock 已接入 SQLite durable intent、filesystem journal、
> exact finalize CAS、request cancellation 与 startup recovery，`N1-4-01` 已关闭。
> 用户针对 exact linked Mount 的 `auto_compatible_when_idle` standing authorization、
> Host-owned eligibility 重算、Runtime lease、resident quiescent fence 与 Apply audit
> 已完成，`N1-4-02` 已关闭。
> Plugin SDK declarations、无用户数据 Share Bundle Export/Import-as-new、来源 Test
> provenance 展示、Desktop Transfer 与 headless import/share/auto-apply CLI 已完成，
> `N1-4-03` 已关闭。
> Plugin N1 Windows Candidate 已在 source commit `a4ddeec70` 完成 18 项安装版
> product smoke，并由 `windows_candidate/plugin_n1` Gate 整体通过；`N1-1-01`、
> `N1-2-03`、`N1-U-01` 与 `N1-V-01` 已关闭。MiniApp M1 Windows Candidate 亦已在
> source commit `cf2f334ff` 完成安装版 UI-only/Service/故障/可访问性 smoke，并由
> `windows_candidate/miniapp_m1` Gate 整体通过；`M1-U-01` 与 `M1-V-01` 已关闭。
> Windows Signed RC 的两个产品 Gate runner 已实现，当前 StepFun Coding Plan smoke
> 再次通过；本机没有 release code-signing certificate，故 `RC-WIN-01` 保持
> `pending-validation`。macOS/Linux 外部原生验证仍未关闭。
>
> Windows 阶段性交接启动材料：
> `CROSS-MACHINE-WORK-START-PROMPT-2026-09-09.zh.md`

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
| 已关闭 | 31 | 当前表内已关闭的 W0/N1/M1 项；Plugin N1 与 MiniApp M1 Windows Candidate 均已关闭 |
| 正在实施 | 0 | 当前 Windows 主线没有 `in-progress` 项 |
| 已解锁待领取 | 0 | 当前 Windows 主线没有未领取实现项 |
| 待 Windows 验证 | 1 | `RC-WIN-01`；等待真实签名身份与 signed artifact/release lock |
| 依赖阻塞 | 1 | `RC-MERGE-01` |
| 外部原生 | 2 | `RC-MA-01`、`RC-LD-01` |
| 明确延后 | 2 | Marketplace/远程分发、第二 Runtime |

## 2026-09-06 实施记录

1. `N1-0-01` 已完成全仓依赖与产品残留扫描：
   - 旧 `/api/presets` 生产可达性为 `0`；
   - 旧 Extension 非 Skill 生产链已随新 Plugin 主链完成迁移并物理删除；剩余命中仅限
     历史删除合同、负向路由测试、通用语义中的 `Extension` 字样，以及新 JavaScript
     Host 的历史内部命名；
   - 旧 MiniApp 数据表仍由 DB/id-schema/backup 合同保留，不能在没有迁移策略和真实
     消费者证据时破坏性删除。
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
| `N1-1-01` | closed | Runtime lane；`nomifun-js-runtime/**`、App composition、Plugin Runtime boundary | Node probe/download、唯一 committed Runtime authority、全局 admission fence、drain/stop/validation/commit-or-restore | `N1-0-02` | Runtime/SQLite/App 定向回归；`a4ddeec70` 安装版完成 committed switch、两次 PID 变化重启、无效路径 typed fault、stale revision 拒绝与重启后 selection 保持；finalizer-before-fence-release 死锁回归已修复 |
| `N1-1-02` | closed | Runtime lane；新 `nomifun-js-host/**` | lazy shared Extension Host、独立 Candidate Test Host、private IPC/Hello、watchdog、whole-tree cleanup、late-result fence | `N1-1-01` | Host 14；role-isolated process、demand=0、crash/restart、child cleanup、quiescent fence、installed-path materialization |
| `N1-1-03` | closed | Kernel+Runtime 边界；`nomifun-js-kernel-adapter/**`、Kernel typed exports、Shared Host API | 普通 Plugin Tool/Context/Resource 的 Node proxy；N1 明确拒绝 Role Provider/Plugin Service | `N1-0-03`,`N1-1-02` | Host 13、Adapter 4、Kernel 23、Agent Platform 16；exact Artifact handle fence；无 Rust/Node 双 Registry |

## N1-2：Package 与数据生命周期

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-2-00` | closed | Artifact lane；`nomifun-plugin-platform/**` | directory/zip containment、canonical digest、immutable CAS staging/publish | `N1-0-02` | 9 tests；tamper/traversal/collision/cancel/idempotency |
| `N1-2-01` | closed | DB lane；migration 067/068/070/071、新 repository | Artifact/Project/Candidate/current/previous/Mount/Operation/retained data、Project display metadata 与 Runtime selection schema/path CAS | `N1-0-02` | `6debcb628` + 070/071；Plugin repository 13、Runtime selection 2、ID schema 20；fresh/restart/direct-SQL guards |
| `N1-2-02` | closed | Plugin platform lane；migration 069、DB repository、owner mutation coordinator | Config、Credential slot binding、KV/CAS、stable `dataDir`、owner mutation lock | `N1-2-01`,`N1-1-02` | `a275ddd97`,`a16cfbeff`；repository 12 + ID/schema 20 + lifecycle 31 |
| `N1-2-03` | closed | Plugin application-service/App lane；`nomifun-plugin-service/**`、`nomifun-app/src/router/plugin_platform.rs`、Runtime-bound Host | staging/containment/digest/install/replace/restore/uninstall/delete-data/project-delete 与真实 App/Kernel/Runtime 组合 | `N1-2-01`,`N1-2-02` | `a4ddeec70` 安装版两代 Source→Build→Test→Apply，真实 Agent→Kernel→Shared Host Invoke 返回 `candidate-v2`，随后 Restore 回第一代 exact Artifact；library revision JSON-safe CAS 与 Plugin artifact identity 回归已覆盖 |

## N1-3：Catalog 与消费者

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-3-01` | closed | Catalog lane | ManagedLocal Package/Mount materialization、五类 Contribution、provenance/availability | `N1-0-03`,`N1-1-03`,`N1-2-03` | 五类 exact materialization；ManagedLocal Skill/MCP binding+Mount+Artifact；duplicate/fault atomicity；Kernel 26、Control Plane 17、Platform 23、JS Adapter 4 |
| `N1-3-02` | closed | Nomi consumer lane | 用动态 action schema/Kernel invoke 替换 Nomi 硬编码 Capability→Tool 表 | `N1-3-01` | content-addressed schema registry；Snapshot-bound provider/session；deferred ToolSearch；Kernel invoke/stale Artifact fail-closed；4 consumer + 1 single-flight + App schema E2E |
| `N1-3-03` | closed | 非 Agent consumer lane | 一个共享 Capability 与一个 non-Agent-only reference contribution | `N1-3-01` | source-neutral operation handler；Agent+Gateway shared Tool、UI-only Tool、exact lock/Artifact drift 与 Agent filtering 均通过 |

## N1-4：Authoring 与 Self-Evolution

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-4-01` | closed | Authoring lane；`nomifun-js-authoring/**` | Source Store、JS/TS scaffold、pure-JS dependency exact lock、fixed packer/Build Host | `N1-0-02`,`N1-1-02`,`N1-2-01` | authoring 42 + public npm live smoke 1；production HTTPS registry/SRI/tar containment、全图 budgets、resolver/cache、共享 registry/build admission、static bundler/Node Host/Artifact admission/cancel、Chat Dev Source edit，以及 SQLite intent + filesystem journal + exact finalize/startup recovery 的 Project dependency mutation 均已通过 |
| `N1-4-02` | closed | Plugin project lane | single Ready、Candidate Test、impact、manual/compatible-idle Apply、Discard、Retry/Restore | `N1-2-03`,`N1-3-01`,`N1-4-01` | migration 085 持久授权/revision；Host-owned 15-predicate eligibility；Runtime lease + non-resident/resident quiescent fence；busy 保留 Ready、quiescent 原子 Apply；不可变 Mount revision 记录 manual/standing-auto 来源；DB 18、Service 31、App route 1、UI bridge/model 13 |
| `N1-4-03` | closed | SDK/CLI lane | Plugin SDK、Share Bundle/prebuilt import/export、本地 CLI | `N1-4-02` | scaffold 固定 SDK declarations；无用户数据目录型 Share Bundle exact Source/lock/Artifact/Test provenance；Import-as-new 进入本机 Ready/Test/Apply；Desktop export/import；CLI import/share/auto-apply + 既有生命周期命令；Authoring 44、Service 34、DB 18+20、App route/CLI、UI 定向与 production build 通过 |

## N1-X：旧 Extension 删除与产品 UI

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `N1-X-01` | closed | Skill lane；新 `nomifun-skill-library/**` | 把 `skill_service`、builtin skills、Skill market 从 `nomifun-extension` 抽为独立 owner | `W0-01` | `610b59007`；Skill Library 150 passed/2 ignored；Extension/消费者 checks passed |
| `N1-X-02` | closed | demolition lane；旧 Extension 生产链 | 删除旧 Extension loader/registry/hub/hot reload/permissions/settings/webui/agent/theme 路径 | `N1-X-01`,`N1-3-03`,`N1-4-03` | `nomifun-extension` crate、旧 `/api/extensions/*`/Hub/生产消费者已物理清理；剩余仅历史删除合同、负向测试和新 JS Host 的历史命名 |
| `N1-U-01` | closed | UI lane；新 `pages/plugins/**`、`pages/settings/RuntimeManager/**` | Plugin Library/Workshop/配置/诊断、Node Runtime Manager；MCP 页面只保留 MCP | `N1-2-03`,`N1-4-02` | targeted/UI production build；安装版 Plugin 窗口 23、Runtime Manager 6 个 visible interactive control 全部具名；两张 PNG 截图经人工视觉检查无明显布局异常 |
| `N1-V-01` | closed | 集成 Owner | Windows N1 contract/integration/fault/product/NSIS candidate | 所有 N1 项 | source `a4ddeec70`；18/18 安装版 smoke PASS；Plugin N1 Windows Candidate Gate PASS，cohort `360adb09d405daac79163883098541d235f5f4a78cf75399017f2318bd5d1e18` |

## M1：Full-stack MiniApp

| ID | 状态 | Owner/写集 | 目标 | 依赖 | 最小验证 |
| --- | --- | --- | --- | --- | --- |
| `M1-0-01` | closed | MiniApp DB/domain lane；`nomifun-db/migrations/072*`、`repository/miniapp_m1*` | 全新 Product/Project/Ready/Active/Previous 数据根，不读旧 `miniapps` | `N1-V-01` | migration 072、schema/repository、生产 owner 路由与 clean-start 数据根定向验证通过 |
| `M1-0-02-A` | closed | MiniApp release lane；`nomifun-miniapp-platform/**`、MiniApp application/App Build adapter | UI-only `miniapp-release-v1` 的 Source→Build→Ready、Build Operation、immutable Release Store | `M1-0-01`,`N1-4-01` | Source owner 隔离、成功/失败/cancel、文件 bytes/digest、Ready lineage/CAS、全程 Node process=0；DB 20+6+8+31、Store 3、Application 3、App route 3、UI 9 |
| `M1-0-02-B` | closed | MiniApp release lane；Publish/Catalog/Surface adapter 与 Desktop Workshop | Ready→Manual Publish→Surface→Rollback、纯 UI auto Publish、Host KV | `M1-0-02-A`,`M1-1-01` | UI-only Release/Catalog 原子切换、Active/Previous pointer CAS、Surface epoch fence、UI 定向验证通过 |
| `M1-1-01` | closed | Service/Bridge lane；`nomifun-miniapp-platform/src/{service_host,service_process,service_runtime}.rs` | 单 `main.mjs`、dedicated Host、on-demand/continuous、MessageChannel epoch fence | `M1-0-02`,`N1-1-02` | 真实 Node NDJSON 3、Service application 1、Runtime candidate 3、旧 generation/崩溃隔离/容量/backoff 通过 |
| `M1-1-02` | closed | Managed data lane；`managed_storage.rs`、Service IPC、M1 cutover | UI/Service KV、Files、Private SQLite、authorizer、参数化 SQL、additive migration ledger | `M1-1-01` | production SQLite Storage 1、真实 Node Storage IPC 3、authorizer/批量回滚/启动与取消边界通过 |
| `M1-2-01` | closed | Lifecycle lane；MiniApp lifecycle/application/data cleanup | Enable/Disable/Trash/Restore/Permanent Delete、Service Test、Share/Backup Import-as-new | `M1-1-02` | 删除恢复、Service Test、Share/prebuilt Import-as-new、Desktop Transfer UI、Whole-App Backup Export/Import-as-new 均已通过 Windows 定向验证；含 Service Files/SQLite/ledger、Catalog digest、owner export mutex |
| `M1-U-01` | closed | UI lane；整体重写 `pages/miniApps/**` | Library/Workshop/Surface，删除 Guid/Conversation 旧 MiniApp 模式 | `M1-0-02`,`M1-1-01` | 安装版 Library 18、UI Workshop/Surface 21、Service Workshop 23 个 visible interactive control 全部具名；三层 viewport/page 横向滚动指标均为 0；截图完成自动取证与人工视觉复核 |
| `M1-V-01` | closed | 集成 Owner | Windows M1 contract/integration/fault/product/NSIS candidate | 所有 M1 项 | source `cf2f334ff`；安装版 17/17 smoke PASS；MiniApp M1 Windows Candidate Gate 10/10 PASS，cohort `b8863b5f7ca8b6dca4411a7ed3860b7c0c14b907f385caedde3f5fd03c7b535c` |

`nomifun-miniapp-platform` 的内存实现仍保留作为合同测试，但不再承担生产事实。当前
生产组合已接入 dedicated Node Service Host、candidate Runtime 验证、Host-owned
Surface/Service Bridge、owner-scoped Files、Host-managed Private SQLite 和持久
Migration ledger。Node Service 的 Storage 请求使用同一私有 NDJSON 通道，数据库路径
不进入 Service SDK/HTTP/renderer wire；Publish 在旧 Host 停止后执行 pending additive
Migration，再启动目标 Host，失败时保留旧 Active 并重建旧 Service。M1-2 生命周期删除
与 Service Test 子切片已完成；Share Bundle Export/Import、source-less prebuilt
Import-as-new 的 Application/API/E2E、Desktop Transfer UI 和 disabled Whole-App Backup
Export/Import-as-new 均已完成。`M1-2-01`、`M1-U-01` 与 `M1-V-01` 现已关闭，当前
已构成 Windows M1 Candidate，但不构成 Signed RC 或跨平台完成。下一边界是
`RC-WIN-01` 的最终 source cohort、签名安装包、StepFun 与 Plugin/MiniApp 联合 release
lock/result。

## 最终候选与外部验证

| ID | 状态 | 目标 | 依赖 | 完成定义 |
| --- | --- | --- | --- | --- |
| `RC-WIN-01` | pending-validation | 冻结全部 Windows 开发的最终 source cohort | `N1-V-01`,`M1-V-01` | final NSIS、StepFun、Plugin/MiniApp installed-app smoke、release lock/result；Gate runner 已齐，当前缺真实签名身份与 signed artifact root |
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

## 2026-09-09 M1-2 Service Test 子切片收口

1. `708ef83b7` 已完成 Service Ready Test：
   - migration 083 持久化 owner-scoped immutable receipt 历史；record CAS 精确锁定
     Product/Pointer/Config/Credential revisions 与 current Ready Release，Retest 只替换
     Ready receipt ref 并保留历史；
   - Test 使用 prospective Active epoch 解析同一 canonical Service spec；Runtime 新增
     `MiniappServiceTestHost` lease，独立 one-shot Node Host 不进入生产 Host 容量表；
   - 生产 Service 在 Test 前停止；KV 复制到一次性 namespace，Private SQLite 使用一致
     快照，Files 为空目录，Ready Migration 只在测试 DB 执行；结束后清理并恢复 Active
     Service；
   - 无正式 callable contribution 时 Host start/stop 可形成 `passed`；需要受管输入时
     返回 `needs_test_input`；Host 失败记录 `failed + error_code`，不伪造通过。
2. Receipt 与 Publish：
   - Workshop 显示 `not_run/passed/failed/needs_test_input/stale`；
   - Runtime、Ready、Product、Config 或 Credential 漂移使 receipt 失效；
   - Publish 使用 passed receipt 时无需警告；failed/needs-input 或无 receipt 时必须显式
     确认，旧 receipt 不作为 fallback。
3. Desktop 已接“测试 Ready Service”动作和风险确认，不要求用户填写 digest、路径、
   generation 或 raw Storage handle。
4. 定向证据：DB repository 20、schema 13、ID schema 20；Platform lib 25、Service Test
   Storage 4、Service application 1、managed storage 2、真实 Storage IPC 3；App route 6；
   Agent contract 21；UI/wire 27。受影响 crate check、generated contract check、i18n、
   rustfmt 和 diff check 均通过。
5. 当前边界：`M1-2-01` 仍未关闭。下一步实现默认无用户数据 Share Bundle、source-less
   prebuilt Import，以及与其分离的 disabled Whole-App Backup Export/Import-as-new。

## 2026-09-09 M1-2 Share Application 子切片收口

1. `a1534ad09`、`47f03ae0f` 已完成 Share Application/API/E2E：
   - 默认无用户数据的 MiniApp Share Bundle Export/Import 已接入；
   - source-less prebuilt Release Import-as-new 已接入；
   - Import 校验、owner 隔离和新 identity 创建已由 application service 与 E2E
     覆盖。
2. 所有导入都创建新的 disabled MiniApp identity，不静默覆盖现有产品；带 Source 的
   Share Bundle 同时创建可继续开发的 editable Project，source-less prebuilt
   Artifact 保持 runtime-only，不伪造可编辑 Source。
3. 当时边界：`M1-2-01` 尚未关闭。该阶段记录的 Share Application/API/E2E 已关闭，
   Desktop Transfer UI 与 disabled Whole-App Backup Export/Import-as-new 随后在
   2026-09-09 收口；`M1-U-01`、`M1-V-01`、Windows NSIS 和 macOS/Linux 原生验证
   状态由后续条目继续跟踪。

## 2026-09-09 M1-2 Desktop Transfer 与 Whole-App Backup 收口

1. Desktop Transfer UI 已完成 Share Bundle、source-less prebuilt Artifact 和
   Whole-App Backup 的导入/导出入口：
   - Share Bundle 与携带业务数据的 Backup 继续使用两个独立产品流程；
   - Backup 导出只允许明确的 disabled MiniApp，导入始终创建新的 disabled
     MiniApp、Project、Release identity，不恢复本机 Credential binding；
   - 普通用户只选择目录和显示名称，不接触 digest、handle、generation 或内部路径。
2. Whole-App Backup 已完成真实 application/API/存储闭环：
   - 固定目录格式、canonical JSON、严格 inventory、digest 校验、大小限制以及
     symlink/junction/reparse/special file 拒绝；
   - Product/Project、Ready/Active/Previous Release、Source、非秘密 Config、KV、
     Service Files、Private SQLite 和 Migration ledger 可导出并导入；
   - 导入后的 Active Catalog digest 按新 MiniApp/Release identity 重新计算；
     Credential slot 必须与保留 Release 的 union 完全一致；
   - Migration ledger 只接受仍在 Backup 保留指针中的精确 Release，不能伪造历史
     Release ref；
   - Files 与 Private SQLite 使用 sibling staging + quarantine 原子交换，失败时
     保留旧目标；导出在同一 storage lock 内捕获 Files/SQLite，普通 Share Export
     与 Backup Export 共享 owner operation 互斥。
3. 定向验证：
   - `cargo test --locked -p nomifun-miniapp-platform --lib backup::tests`：4 passed；
   - `cargo test --locked -p nomifun-miniapp-platform --test backup_application`：
     2 passed（UI-only、active Catalog digest）；
   - `cargo test --locked -p nomifun-miniapp-platform --test backup_service_application`：
     1 passed（Files、Private SQLite、Migration ledger、Release 重绑定）；
   - `cargo test --locked -p nomifun-miniapp-platform --test managed_storage
     --test service_test_storage --test share_application`：7 passed；
   - `cargo test --locked -p nomifun-db --test miniapp_m1_repository
     --test miniapp_m1_schema --test id_schema_contract`：定向通过，包含普通
     Export/Backup operation 互斥；
   - MiniApp UI/wire 定向测试 35 passed，`check:i18n`、UI production build、
     受影响 crate `cargo check`、定向 rustfmt 和 `git diff --check` 通过。
4. `M1-2-01` 现已关闭。`M1-U-01` 的 Desktop 产品/accessibility 走查、
   `M1-V-01` 的 Windows Candidate/NSIS/fault 验证仍未关闭；macOS arm64、
   Linux Desktop x64 和手机模式均不在本机当前验证范围。

## 2026-09-09 跨机器阶段性交接

当前 Windows N1/M1 主线已在 `51b0243f7` 形成可交接基线。用户要求先完成稳定的
阶段性收尾，再由另一台更快的 coding agent 继续实现；启动顺序、已交付事实、当前
代码勘察点、最小验证命令、StepFun Credential Manager 规则和 Git 提交流程统一记录在
`CROSS-MACHINE-WORK-START-PROMPT-2026-09-09.zh.md`。

本次交接不改变本台账的状态含义：`M1-2-01` 已关闭，`M1-U-01`、`M1-V-01`、
`N1-V-01` 和 `RC-WIN-01` 仍未关闭；MiniApp Active Release 的 shared Catalog
consumer integration、旧生产链物理清理和 Windows Candidate 仍须按本文及 06 的依赖
顺序推进。手机模式、macOS arm64 和 Linux Desktop x64 不在当前 Windows 交付范围内。

## 2026-09-09 MiniApp shared Catalog publication 一致性收口

本轮完成 MiniApp Active Release 到共享 Formal Capability Catalog 的第一阶段接入。
publication update 使用 owner、Product/Pointer revision 和 Active Release epoch，
并用版本 tombstone 防止迟到 Publish/Disable/Delete 回调复活旧能力。
`catalog_digest` 现在覆盖排序后的完整 `CapabilityCatalogPublication`；Product digest
漂移时启动 hydrate fail closed。MiniApp capability 合同 owner 仍是 Package，产品
生命周期 owner 仍由 application service 的 `owner_user_id + miniapp_id` 校验。
尚无真实执行适配器的 Agent/Gateway consumer 保持显式 unavailable；已落地的
MiniAppService consumer 才可标记 active。因此本轮不关闭 Agent dispatch、Gateway
invoke、旧生产链清理或 Windows Candidate。

定向证据：Agent contracts 84、Control Plane 22、Agent Platform 18、MiniApp Platform
29、MiniApp backup application 2、MiniApp DB repository/schema 22 + 13；受影响 crate
check 和 `git diff --check` 通过。全量 rustfmt 仍受 Windows 文件名长度限制，使用
受影响 crate 定向检查替代。

## 2026-09-09 MiniApp Agent capability port 第一阶段

`MiniAppAgentCapabilityPort` 已在 MiniApp application service 内落地。它只允许
enabled Service MiniApp 的正式 Active Release capability，调用前重新校验 owner、
Release identity、epoch、artifact digest、完整 publication digest、Agent/Service
consumer 声明和 action allowlist，再通过现有 dedicated Service Host 执行
`CapabilityActionDescriptor.action_id`。typed resource requirement 当前明确返回
`CAPABILITY_RESOURCE_BINDING_UNAVAILABLE`；不复用 Surface Bridge、不进入 Kernel
Plugin Registry，也不授予 MiniApp owner 权限。

定向验证增加 Service application stale Catalog digest fail-closed 测试并通过。

## 2026-09-09 MiniApp Agent 动态 Tool session 第一阶段收口

Control Plane/Kernel 已增加独立 `ResolvedMiniAppCapability` exact projection：
MiniApp capability 不进入 Kernel Plugin Registry，也不复用 `ResolvedCapability` 的
Plugin mount 字段。Snapshot 现在冻结 MiniApp ID、Active Release/epoch、catalog
digest、contribution lock、action schema/action allowlist 和 required resource kinds；
on-demand activation plan 与 compact index 同样纳入 Snapshot/runtime profile digest。

Nomi-core 通过现有 session provider 将 Plugin 和 MiniApp action 合并进同一个
动态 Tool session；MiniApp action 使用独立 provider 名称和 invoker，schema 由
owner-scoped Release Store 校验，调用最终进入 `MiniAppAgentCapabilityPort` 和
dedicated Service Host。Plugin 行为没有改走 MiniApp 路径，Active Release、catalog
digest、epoch、owner、action 和 typed resource 漂移均 fail closed。

定向验证：Kernel 28、Control Plane 22、Nomi Tool consumer 5、MiniApp Platform 29
及既有 application/backup/service suites 通过；contract generator `write/check`、
受影响 crate check、定向 rustfmt 和 `git diff --check` 通过。该子切片仍不关闭
真实 callable contribution 产品 E2E、`M1-U-01`、`M1-V-01`、`N1-V-01` 或
`RC-WIN-01`；旧 MiniApp/Extension 生产链清理和 Windows Candidate 继续后置。

## 2026-09-09 AgentPreset 启动入口模型边界修正

Guid 入口现按 05 §15.5/15.6 执行：默认 Nomi 保留模型选择器；AgentPreset/官方
模板模式隐藏模型选择器，创建 Session 时只发送 `preset_id` 和 `title`，服务端从
稳定 Revision/Snapshot 取得冻结模型。官方模板准备可省略模型并由 default Chat
route 完成首次配置；普通 Nomi 没有模型时仍明确阻断。

`gate:agent-v2 --self-test`、Guid 结构/行为测试和 UI production build 已通过；
全量 UI typecheck 的既有基线错误仍未纳入本轮。该项是产品入口纠偏，不关闭
`N1-U-01`、`M1-U-01` 或 Windows Candidate。

## 2026-09-09 Windows Candidate 前置检查与旧链安全审计

本机静态/自测试前置均已通过：Agent-v2/N1 gate self-test、Candidate smoke harness
测试、release-lock 测试、NSIS source contract 和 `gate-plugin-n1` contract dry-run。
`windows_candidate --scope combined --cohort local-preflight --dry-run` 明确列出
18 个 required product/integration/fault checks 仍为 `pending`，因此不关闭
`N1-V-01`、`M1-V-01` 或 `RC-WIN-01`。

旧链审计结论：旧 Extension 生产链已经物理清理，当前没有可恢复的兼容入口；剩余
Extension 命中仅属于历史删除合同、负向 404 测试、通用语义或新 JavaScript Host
历史命名。旧 MiniApp schema 仍由 DB/id-schema/backup 合同保留，不能在缺少迁移策略
时破坏性删除。`N1-X-02` 已关闭，旧 MiniApp physical cleanup 继续按独立数据迁移
策略跟踪。

全量 UI typecheck 仍有既有 Arco/隐式 `any` 基线错误；生产 UI build 和本轮 Guid
定向测试通过。下一步应由具备安装器/Provider 主机权限的环境继续 Candidate，而不是
在本机伪造 release evidence。

## 2026-09-10 Candidate discard、Windows 安装版基础 smoke 与当前边界

本轮完成并提交 `0e57c3a9c`：

1. Plugin Candidate discard 已形成完整最小闭环。`POST
   /api/plugin-projects/{project_id}/candidate/discard` 使用单个 SQLite transaction
   校验 owner、Project revision、Build generation、Ready Candidate ID/digest，清除
   `ready_candidate_id`，删除 Candidate Test receipt 与 Ready Candidate，保留
   Build generation、Artifact 和 origin Operation。CLI 新增
   `plugin candidate discard <PROJECT_ID>`。
2. Candidate discard 定向证据通过：DB Plugin N1 `14 passed`、Plugin application
   service `18 passed`、App route `1 passed`，CLI parse/adapter 测试通过。
3. 当前 HEAD `0e57c3a9cba5ae3cecabcf323f862562c9f2d40c` 已构建新鲜 Windows x64
   NSIS `NomiFun_0.7.6_x64-setup.exe`；真实安装版基础 smoke 通过 `14/14`，覆盖
   安装、x64 binary、启动、port announcement、backend health、WebView2 CDP、
   进程树清理、卸载和注册表/安装目录清理。该 smoke 不等于 Plugin/MiniApp
   生命周期或 fault Candidate 通过。
4. Windows Credential Manager runner 已完成一次真实 StepFun Coding Plan
   `step-3.7-flash` smoke，结果为 `live_smoke_status=pass code=OK status=200`；
   凭据未进入源码、argv、日志、fixture 或 Git。该 smoke 只证明 Nomi-core Provider
   链路，不替代 Plugin/MiniApp Candidate。
5. 当前仍未关闭：`N1-1-01` 的安装版 Runtime switch/restart/fault 证据、
   `N1-2-03` 的安装版 Plugin Build→Test→Apply→Invoke→Restore、
   `M1-U-01` 的真实 Tauri 产品/accessibility 走查、
   MiniApp callable 完整产品 E2E、`N1-V-01`、`M1-V-01` 和 `RC-WIN-01`。
6. 当前生成的安装包只作为本机候选输入，尚未把未完成的产品/fault checks 伪装成
   Gate PASS；历史 `0.7.4` 安装包禁止复用。手机模式、macOS arm64 和 Linux
   Desktop x64 仍不在本机范围内。

## 2026-09-10 Plugin production npm registry adapter 子切片

1. `N1-4-01` 的 production registry adapter 已形成：只访问配置的同源 HTTPS registry，
   不跟随重定向；metadata 和 tarball 都有固定大小/超时边界。同步 transport 不创建私有
   Tokio runtime，可安全进入后续 application-owned blocking worker。
2. tarball 在解包前校验 registry `sha512` SRI，解包只接受 canonical `package/` 根下的
   普通文件，并拒绝 traversal、非 UTF-8、Windows case/NFC collision、symlink、hardlink、
   special file、native addon、lifecycle script、optional/peer/bundled/platform install
   surface。SemVer 解析继续生成 content-addressed cache 和 exact transitive lock；
   每次 resolution 另有 package/file 数、递归深度、累计传输/解包 bytes 和总耗时预算，
   递归时不再保留已缓存 Package 的完整文件 bytes。
3. registry admission 与 fixed bundler 现在复用同一个 package.json 校验器：普通 author、
   license、repository、devDependency 与非 lifecycle test/lint script 不造成 resolve/build
   漂移；CommonJS、lifecycle/native/optional/peer/bundled/platform surface 在写 lock 前拒绝。
4. 定向证据：`nomifun-js-authoring` 36 passed / 1 ignored；被 ignore 的 public npm
   `yocto-queue@1.2.1` live smoke 已显式运行并通过。受影响 crate check、定向 rustfmt、
   contract generator write/check 和 `git diff --check` 通过。
5. 本轮审查曾验证一个直接写 `package.json + dependency-lock.json` 的原型，但发现 HTTP
   取消可在 blocking worker 与 DB CAS 之间形成分裂，多次 rename 也没有 crash journal/
   startup recovery；该原型及其 API/UI 已撤回，没有把不安全的多文件提交暴露给用户。
   `N1-4-01` 因此在该子切片结束时保持 `in-progress`；随后已按下一节先定义 durable
   mutation intent 与 commit/recovery/finalize 顺序，再接 application/API/Desktop。

## 2026-09-10 Plugin durable dependency mutation 收口

1. `N1-4-01` 已关闭。Plugin Project 的完整直接依赖映射现在通过唯一产品 API 更新；
   request 必须携带 Project revision、Build generation、Source digest 与 lock digest
   的 exact CAS。相同映射是无网络、无 generation 变化的 no-op；真实变化先在私有
   staging 完成 npm resolution、SRI/admission 与新 Source/lock digest 计算。
2. 提交顺序已固定为：SQLite 写入 durable mutation intent 并用 trigger fence 普通
   Project UPDATE/DELETE；Source Store 持久化 canonical filesystem journal；交换
   `package.json` 与 Host-owned `dependency-lock.json`；SQLite 在单 transaction 内按
   intent exact finalize 并把 Build generation 加一；最后删除 staging/backups/journal。
   不再使用无恢复协议的多文件 best-effort rename。
3. request 取消只在 registry/resolver/staging 阶段生效，取消会清理 staging，且不会写
   intent 或 live Source。进入 durable intent 后即使 HTTP future、进程或文件交换中断，
   启动恢复和下一次 Project 读取都会按 DB old/new facts 确定性 rollback 或 finish；
   orphan staging 只在与 DB intent/journal 集合核对后清理。
4. migration 084 新增 owner/project-scoped intent、短生命周期 finalize marker 和五个
   guard/cleanup trigger；普通仓储更新与直接 SQL 都不能越过 pending intent。测试还
   人工模拟了 journal 已落盘但文件只完成第 1 步或第 3 步交换后的重启，两种状态均
   精确恢复原 Source+lock。
5. Desktop Workshop 已提供“编辑依赖”对话框，预载当前完整直接依赖映射，只提交
   SemVer string map 和 exact CAS；失败保留表单，成功刷新 Project/Library。生产组合根
   使用 hardened npmjs client 和 content-addressed cache，并在暴露 Router 前执行恢复。
6. 定向证据：`nomifun-js-authoring` 42 passed / 1 public-registry test ignored（该
   `yocto-queue@1.2.1` live smoke 已在前一子切片显式通过）；DB Plugin repository 16、
   ID/schema contract 20、Plugin Service 30、App route 1、dependency UI bridge/dialog 9
   均通过；`check:i18n`、UI production build、contract generator check、定向 rustfmt
   与 `git diff --check` 通过。App 的非定向 integration test 编译仍会命中仓库既有缺失
   `tests/extension_e2e.rs`，本轮使用 `--lib` 精确执行并通过目标路由测试。

## 2026-09-10 Plugin compatible-when-idle auto Apply 收口

1. `N1-4-02` 已关闭。migration 085 把 `ask_before_apply |
   auto_compatible_when_idle`、exact authorized Mount、单调 authorization revision 与
   authorized time 写入 Plugin Project；Project 初始状态永远无授权，启用必须同时通过
   owner、Project revision/Build generation、linked Mount revision/current digest 的
   exact CAS，关闭授权只要求 Project CAS，Mount 故障时仍可撤销。
2. eligibility 不再由调用者提交。Application service 在 owner linked-mutation lock 内
   从 SQLite Project/Candidate/TestReceipt/Mount、immutable Artifact Store 和 committed
   Runtime 重算合同冻结的 15 个 predicate，包含 authored Source/lock lineage、Candidate
   base/current、完整 receipt/runtime digest、Contribution/Config/Credential/Resource/
   Effect/Host SDK/Runtime/platform/dependency lock、静态校验和 unknown-facts fail-closed。
   runtime-only、首次安装、Breaking、dependency-lock 变化和任一 stale/unknown 均保留
   Ready 并转人工。
3. 真正提交前取得持有到 DB commit 结束的 Runtime read lease；非 resident Mount 不启动
   Node。resident Mount 使用 Host admission write fence 原子阻止新请求：有 in-flight/
   queued 调用时返回 busy、保持 Ready 且绝不进入人工 stop/cancel；quiescent 后停止并
   reap 整代，再由同一 SQLite transaction 轮换 current→previous、Candidate→current
   并只清除 exact Ready/TestReceipt。下次真实 demand 才按新 current 冷启动。
4. auto Apply 在匹配 Candidate Test 完成、用户启用/重试授权和 App 启动时事件驱动尝试，
   不建设 polling updater。`plugin_mount_revisions` 的不可变记录新增
   `manual_user_confirmation | standing_auto` 与 exact authorization revision，构成 Apply
   审计；Desktop 在自动成功时给出非阻断通知并继续提供 Restore Previous。
5. Desktop Workshop 已增加启用、关闭与“立即重试自动应用”入口。首次启用明确提示：
   contract-compatible 不保证行为、费用、网络副作用或 dataDir 兼容，且连续 Apply 只
   保留一代 previous。Candidate blocking reason 使用本地化分组，不向用户显示内部
   predicate 字段名。
6. 定向证据：DB Plugin repository 18（含持久授权、stale CAS、data-delete revoke、
   auto audit）、ID/schema contract 20；Plugin Service 31，其中 auto Apply 2 覆盖
   “授权但未 Test 不应用”、非 resident 自动应用、resident busy 保留 Ready、quiescent
   事件后应用且从不进入 manual stop；真实 Node Host resident fence 1；App 组合路由 1；
   UI bridge/model 13、dependency dialog 2、i18n 与 production build 通过。完整安装版
   Build→Test→auto Apply→Invoke→Restore 仍属于 `N1-V-01`，没有提前记为 Candidate PASS。

## 2026-09-10 Plugin SDK、Share Bundle 与 headless CLI 收口

1. `N1-4-03` 已关闭。新建 JavaScript/TypeScript Project 固定包含
   `nomifun-plugin-sdk.d.ts`，声明 Host 注入的 exact Mount target、Credential resolve
   与 owner-scoped State get/set/delete/compareAndSwap；TypeScript scaffold 直接使用
   `PluginActivationContext`，declaration 不进入运行 bundle，也不会被当作未引用可执行
   Source。运行时仍只有现有 Host 注入 SDK，不建立第二套 loader/Registry。
2. Plugin Share Bundle 使用固定目录 inventory：canonical `bundle.json`、immutable
   Artifact record/package，以及可选 canonical Source snapshot、Host-owned dependency
   lock 和逐文件 bytes。Export 只允许 exact Ready Candidate，或 exact linked current
   Mount；只有 Ready 的 Source/lock/build lineage 仍等于 Project head 和 Artifact lock
   时才能带 Source，current Mount 不猜测已经漂移的作者 Source。
3. Bundle import 对文件与空目录都执行 exact-set、file/total/count/path budget、Windows
   case/path 规则、symlink/junction/reparse/special file 拒绝，逐项重算 Source、lock、
   Manifest 与 Artifact digest，并再次通过正式 Artifact Store admission。Source
   package identity 必须等于 Artifact；任何 tamper、额外 `credentials.json`、额外空目录
   或 lineage 分裂都 fail closed，目标目录存在时不覆盖，失败 staging 自动清理。
4. Import 始终创建新的 Project identity：带 Source 的 Bundle 创建 generation=1 的
   editable Project，source-less Bundle 创建 runtime-only Project；两者都只生成 Ready
   Candidate，不安装、不启用。来源 Test provenance 只持久化/展示 outcome、Candidate
   digest、Runtime target/executable digest 和 Host/SDK/Test contract version，不含测试
   输入、输出、API response 或日志，也不能满足本机 Test/auto-Apply eligibility。
5. migration 086 为 Product Operation 持久化 bounded result Artifact digest map，并以
   insert/update trigger 保证 Operation 从空结果开始、只有成功状态可发布 SHA-256 facts；
   Ready Candidate 可保存仅属于 import 的来源 Test provenance。Share Export/Import
   Operation 返回 `share_bundle` 与 `package` digest，可审计且不暴露本地内部路径。
6. Desktop Workshop 新增 Ready/current Share Export，明确不包含 Credential bindings、
   Config、KV、dataDir、Files 或测试内容；原 Import 对话框新增 NomiFun Share Bundle
   类型。Headless CLI 新增 `plugin import [--share-bundle]`、`plugin share export` 和
   `plugin auto-apply enable|disable|retry`，继续只调用同一 HTTP application service，
   不直开 SQLite/Source Store 或绕过 Candidate。
7. 定向证据：`nomifun-js-authoring` 44 passed / 1 public npm test ignored（live smoke 已在
   前序切片显式通过）；Plugin Service 34，其中 Share filesystem/application 3；DB
   Plugin repository 18、ID/schema 20；App 组合路由 1 覆盖 Share Export→Import-as-new，
   CLI parse/help 2；UI bridge/model 15、Share dialog 1、dependency dialog 2、i18n 与
   production build、contract generator write/check 与定向 Clippy 通过。安装版一站式 Build→Test→Apply→Invoke→Restore/Share smoke
   继续归 `N1-V-01`，不以 crate 测试冒充 Candidate PASS。

## 2026-09-11 Plugin N1 Windows Candidate 收口

1. `N1-1-01`、`N1-2-03`、`N1-U-01` 与 `N1-V-01` 已关闭。最终冻结 source
   commit 为 `a4ddeec70643d6b9fd2fd719f23be289c9d93048`；NSIS 安装器 SHA-256 为
   `93fbe4216c4c4c69909942f62b026f5bf629729ffdab8ffc05834479185dcd52`，安装后
   x64 主程序 SHA-256 为
   `a0a9ecc7bb9d799cf5b268eb9671b932423cc2f11727a74e8cfb2254e026b57c`。
2. 安装版 product runner 共 18 项全部 PASS：干净 source checkpoint、隔离安装、x64
   二进制、启动/port/health/WebView2 CDP、installation-token 控制面、Runtime
   switch/restart/fault、Plugin lifecycle/Invoke/Restore、Desktop a11y、精确进程树清理、
   静默卸载和注册表归零。canonical evidence 位于
   `build.noindex/windows-candidate/a4ddeec70/product-runs/mtvr04gd-17as`。
3. Runtime 使用复制到隔离数据根的真实 Node 24.18.0 完成 committed switch；提交后和
   fault 后各重启一次 Desktop，PID 均变化且 selection digest 保持。无效路径产生
   `NODE_PROBE_FAILED`，stale revision switch 返回 typed conflict。实跑发现并修复了
   committed CAS 后仍持 write fence 调用 availability finalizer 的 read/write 自死锁；
   新回归测试直接让 finalizer 反向读取 committed Runtime，必须在 1 秒内完成。
4. Headless 产品控制面现在接受 installation Bearer 并只绑定 canonical installation
   owner；错误 token、普通 JWT 对 Plugin local-product surface、以及 installation token
   对 Agent Catalog 均 fail closed。非 ambient Bearer 正确跳过 cookie CSRF，但有效性和
   scope 仍由后续 auth/owner/local-product middleware 决定。
5. Plugin Project 使用两代同版本不同 digest Artifact：第一代
   `886730583386df4fbc20fb49cc528796e8fbbb4076f79cf847996c45b9b389eb`，第二代
   `4f80ad9c740b3d11283396239fbad4a2a091bc9d28a0aa551c1d9099c6ff1d14`。两代均完成
   Build/Test/Apply；本地 deterministic OpenAI-compatible mock 只决定 Tool call，不伪造
   结果，真实 Agent→Kernel→Shared Host Invoke 返回 `candidate-v2`，随后 Restore 把
   current/previous 精确交换回第一代。
6. 实跑另修复两个只会在 JS 产品边界出现的问题：content-derived Plugin library
   revision 现限制为非零 53-bit JSON-safe integer，避免 Desktop/CLI 往返丢精度导致
   `PLUGIN_STALE`；Plugin/MiniApp Tool 的完整 activation identity 与 artifact semantic
   identity 已分离，普通 JSON Tool 不再因 provenance 中固定存在 `package`、
   `artifact_digest` 而被误判为文件产物，exact activation digest 锁保持不变。
7. Desktop 安装版可访问性扫描覆盖 Plugin 工作台 23 个、Runtime Manager 6 个 visible
   interactive control，均有 accessible name。截图
   `evidence/plugin-workshop.png`（SHA-256
   `7f8a76e2c62763913a7ada443c510e6fb5cbc4a83ce9bbf0df9db700d4520bf5`）与
   `evidence/runtime-manager.png`（SHA-256
   `854b1e83c56cd733cd6e1931d4ec1ae4521db72633807fc2bf99c20a449936c5`）经人工检查
   无明显重叠、不可辨识控件或状态层级异常。
8. Plugin N1 `windows_candidate/plugin_n1` Gate 整体 PASS，canonical result 位于
   `build.noindex/plugin-n1-gate/n1-win-a4ddeec70/windows_candidate/plugin_n1-windows_desktop_x64.result.json`，
   cohort digest 为
   `360adb09d405daac79163883098541d235f5f4a78cf75399017f2318bd5d1e18`。MiniApp M1
   installed product/a11y、最终 Signed RC 与 macOS/Linux 原生 Gate 继续保持未关闭，
   不由本次 Plugin N1 Candidate 代替。

## 2026-09-11 MiniApp M1 Windows Candidate 收口

1. `M1-U-01` 与 `M1-V-01` 已关闭。最终冻结 source commit 为
   `cf2f334ff76c63dd34b3eaeffbb800f4819b7130`；Windows x64 NSIS 安装器 SHA-256 为
   `d6f58ab9de34c01faf1f5a73b2c03552715173304c6288ad6a8f45533581ed65`。
2. 安装版 product runner 共 17 项全部 PASS：干净 source checkpoint、隔离安装、x64
   binary、启动/port/health/WebView2 CDP、UI-only Create→Build→Publish→Enable→Surface、
   Host KV、Source 编辑→第二次 Build/Publish→Rollback、Share/Backup Import-as-new、
   Trash/Restore/Permanent Delete、Service Test/Publish/Bridge、强杀 Node→Failed→Retry 新
   PID、stale Surface 拒绝、Desktop restart、进程树清理、静默卸载与注册表归零。最终
   product evidence 位于
   `build.noindex/windows-candidate/cf2f334ff/miniapp-product-runs/mtvxuuuw-11kc`。
3. MiniApp Source 编辑不再绕过数据根：migration 087 记录 exact Product/Project/source/
   generation intent 与一次性 commit marker，SQLite trigger 在 intent 期间栅栏普通
   Project/Product/Build 变化；Source Store 原子切换 head 后由数据库 finalize，启动恢复
   能确定性区分旧 head 撤销与新 head 补提交。Desktop Source dialog 和 HTTP bridge 只
   替换现有 UTF-8 受管文件，携带完整 CAS。
4. 实跑修复了两个仅在安装态长路径/故障注入中暴露的 Service 缺口：Windows
   `CreateProcessW` 不接受超长 `lpCurrentDirectory`，长 Release 路径现在只把 cwd 回退到
   已验证 Node 目录，模块仍按绝对路径、Release identity 与 digest 加载；后台
   maintenance 现在主动读取 process completion，外部强杀的 on-demand Service 不再长期
   停留在虚假的 Ready 状态，而是释放容量并进入 Failed，显式 Retry 使用新 PID。
5. Desktop a11y 扫描覆盖 Library 18 个、UI Workshop/Surface 21 个、Service Workshop
   23 个 visible interactive control，全部有 accessible name。document、layout content 与
   MiniApp page 三层均满足 `scrollWidth == clientWidth`、`scrollLeft == 0`；MiniApp 页显式
   禁止横向滚动残留。最终 Gate 截图为 `evidence/miniapp-library.png`（SHA-256
   `bde9617662f23b7b39d5f24202a916852d382fe522fec9f5874ec182d3af3d44`）、
   `evidence/miniapp-workshop-surface.png`（SHA-256
   `833745a8545245e74d716ff7c5dd69d6e17c2f46265940d25691888c0bf2f80c`）和
   `evidence/miniapp-service-workshop.png`（SHA-256
   `5b5273cb1e53fb4ed0f702269e91e91d6f5c231021b672a8e3e6250a986d374e`）。
   WebView2 对 sandboxed OOPIF 的 surface capture 偶发把外层合成图横向偏移，因此 runner
   同时等待 iframe 退出 `aria-busy`、双 animation frame，并使用 view capture；人工视觉
   复核另以同一 UI artifact 的完整稳定帧确认无真实裁切、重叠或不可辨识控件。
6. MiniApp M1 `windows_candidate/miniapp_m1` Gate 10/10 PASS，canonical result 位于
   `build.noindex/plugin-n1-gate/m1-win-cf2f334ff/windows_candidate/miniapp_m1-windows_desktop_x64.result.json`，
   cohort digest 为
   `b8863b5f7ca8b6dca4411a7ed3860b7c0c14b907f385caedde3f5fd03c7b535c`。
   `RC-WIN-01` 的依赖现已全部满足；Signed RC、同 cohort release lock/result、
   macOS arm64 与 Linux Desktop x64 仍未关闭。

## 2026-09-11 Windows Signed RC Gate 实现与当前外部条件

1. 提交 `ed87c3437` 已实现 Signed RC 阶段最后两个 pending product cell；
   `windows_signed_rc/combined` 的 12 个 required checks 现在全部拥有真实命令，不再以
   `check_runner_not_implemented` 阻塞。
2. 新 runner `run-windows-signed-rc-product.mjs` 只接纳当前 clean source 下
   `build.noindex/windows-signed-rc/<HEAD-short>/artifacts` 的真实制品。Host 与 NSIS 均须为
   非空普通文件并具有 `Valid` Authenticode signer 与 timestamp certificate；release lock
   必须逐文件重算通过，且 source commit、`x86_64-pc-windows-msvc`、Host path 与 package
   path 全部精确一致，随后才允许调用既有 Plugin/MiniApp installed-product runner。
   `NOMIFUN_WINDOWS_SIGNED_RC_ROOT` 只允许覆盖到本仓库 `build.noindex` 子目录，不能把外部
   任意文件冒充 RC。
3. Candidate resolver 新增受约束的 `NOMIFUN_WINDOWS_CANDIDATE_ROOT`，只供上述已验证
   signed root 复用既有隔离安装/启动/故障/卸载 harness；普通 Candidate 默认路径和
   provenance 检查保持不变。Signed runner/self-test、基础安装 harness 14 项测试、Gate
   self-test 和 Signed RC combined dry-run 均通过。
4. 当前 source 的真实 StepFun Coding Plan `step-3.7-flash` smoke 已从 Windows Credential
   Manager 隔离执行并再次返回 `live_smoke_status=pass code=OK status=200`；secret 未进入
   argv、源码、日志或 Git。
5. 本机 `WINDOWS_CERTIFICATE_THUMBPRINT` 与 `TAURI_SIGNING_PRIVATE_KEY` 均未配置，当前用户
   Code Signing certificate 数量为 0，因此不能诚实生成 release-grade signed Host/NSIS。
   runner 对两个 scope 均以 `signed_rc_root_missing` fail closed，预期路径为
   `build.noindex/windows-signed-rc/ed87c3437`。`RC-WIN-01` 据此改为
   `pending-validation`，不得使用 unsigned Candidate、自签名临时证书或复制旧制品关闭。
