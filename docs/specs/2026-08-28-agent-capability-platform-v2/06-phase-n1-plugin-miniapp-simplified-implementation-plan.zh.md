# NomiFun 二期：Plugin 与 Full-stack MiniApp 简化实施计划

> 状态：**IMPLEMENTATION AUTHORIZED / AP-0～AP-7 与一期 Windows C8 已通过 / Windows N1→M1 执行中**
>
> 文档日期：2026-09-02
>
> 文档地位：本文是 Phase N1 Plugin 与后续 MiniApp 的设计与实施入口；AgentPreset 的产品和数据合同以 05 §15 为唯一前置来源，本文不得重新定义第二套 Agent 设定或旧 Preset 模型。
>
> 生效边界：05 §15 的 AP-0～AP-7 已通过，用户已授权 Plugin/MiniApp 代码开发。
> 当前先在 Windows x64 完成 N1 Plugin 与 M1 MiniApp，再冻结新的最终候选；macOS arm64
> 与 Linux Desktop x64 统一在全部 Windows 开发完成后交接原生验证。手机模式不属于
> `nomifun-desktop` 范围。

## 2026-09-06 实施启动修订

本节覆盖本文中仍写成“AP 阻断”或“Plugin 三平台先完成再开发 MiniApp”的旧进度口径，
但不改变已经确认的产品合同：

1. AP-0～AP-7 已关闭；一期 Windows C8 候选
   `0bac72da4ebb62f6a0f183a1285065c88aa684a4` 已通过，N1-0 可以进入代码实施。
2. 代码依赖仍保持 **N1 Plugin 先于 M1 MiniApp**；但验证顺序改为
   **N1 Windows 完成 → M1 Windows 完成 → 冻结最终 Windows 候选 →
   macOS arm64 与 Linux Desktop x64 统一交接**。
3. N1/M1 实时状态只由 `PHASE-N1-M1-CLOSURE-TODO.zh.md` 维护。本文继续作为设计合同，
   不在章节叙述中重复维护百分比。
4. 当前 `nomifun-extension` 与旧 MiniApp 是待删除的旧产品主链，不是 N1/M1 半成品；
   必须先迁出真实 Skill owner，再完成新主链并物理删除旧 Extension/MiniApp 兼容路径。
5. Windows 验收只覆盖真实 Desktop x64、NSIS 安装版、键鼠桌面布局和 accessibility，
   不运行手机视口。模型相关 Chat Dev/E2E 固定使用 Credential Manager 中的 StepFun
   Coding Plan `step-3.7-flash`，凭据不进入源码、日志、argv 或 Git。
6. 2026-09-06 的 N1-0 实现已把 `contribution_id` 收归唯一 `CapabilityManifest`，
   `plugin-package-v1` 直接嵌入 canonical `PackageManifest`，entrypoint 只有
   `in_process | javascript` 一套联合类型。Kernel/Control Plane 不再从 capability ID
   建立第二个 contribution identity，也不再要求无关全局 Registry generation 完全相等；
   Snapshot 与 invoke 改为比较 exact contribution/Mount/Source/Artifact。
7. Plugin Host wire 已按 Shared Extension、Candidate Test、Build 三种 role 分开，
   Hello 不是普通 RPC；请求和响应绑定 role、direction、generation、request ID 与
   exact target。MiniApp M1 使用独立 Service/Bridge 合同，不把 Service 方法塞回
   Plugin Host method set。
8. 独立 `MiniAppM1ContractManifest` 与 `miniapp-release-v1` 已进入同一个 contract
   generator。Bridge 的 Web 内容只发送 call ID、method/payload 或 KV 操作，MiniApp、
   Active Release、epoch、owner 与 Surface Session 均由 Host-owned MessageChannel
   port 绑定；不存在 localhost port/token wire。
9. Windows Node foundation 已实现手工/PATH/managed 三类 probe 与全局试切换。
   Managed 下载只接受用户确认后的 Node 官方 LTS，校验官方 SHASUMS 后发布；
   macOS/Linux archive 实现和原生验证仍按本文总顺序延后，不影响 Windows 主线继续。
10. Windows Shared Extension Host 已使用 stdio NDJSON 私有 IPC 实现 lazy demand、exact
    Hello、Mount 按需加载、request cancel、generation fence、watchdog 与整进程树回收。
    quiescent stop 通过 Host admission 读写锁与请求入队原子互斥，不使用“先观察空闲再停止”
    的竞态实现。
11. Plugin Artifact Store 已实现 directory/zip 两种同源 admission，严格拒绝 traversal、
    symlink、特殊文件、Windows case collision、非 NFC 路径、额外 published root 内容和
    digest tamper；同 `package_id + version` 的不同 digest 可并存，是否 Replace 只在后续
    Candidate/Apply 用户动作中裁决。
12. 产品 API DTO 只暴露 Runtime、Plugin/MiniApp Library/Workshop、Config、
    Credential reference、Candidate/Release、Operation 与明确命令。Managed Node 下载
    只接受服务端 official-LTS offer digest 的确认，客户端不能自报版本、target 或 release
    digest；Bridge、process、generation、ledger 和 secret 不进入 HTTP wire。

## 2026-09-08 M1-0-02-A 实现收口与 M1-0-02-B 进入条件

`M1-0-02-A` 已在 Windows Desktop 主机完成实现与定向回归，产品边界仍严格停留在
UI-only `Source → Build → Ready`。远端 AgentPreset/会话模型选择修正已先同步到当前
分支，MiniApp 的 Build lineage migration 因远端新增 `076_agent_session_model_configurations`
顺延为 `077_miniapp_build_operation_lineage`；正式 migration head 为 `77`。

本轮实际交付：

- MiniApp 专用 owner/project Source Store 和 immutable Release Store，不复用 Plugin
  Source/Package lifecycle，也不读取旧 `miniapps`；
- 固定 `miniapp-release-v1` UI-only builder、Build Operation、single-flight、取消/
  失败终态和 staging cleanup；
- Artifact、Release、Ready pointer、library revision 与 Operation succeeded 的 SQLite
  原子提交，以及 exact source/lock/build-generation/CAS lineage；
- App owner/local-trust 路由和 Desktop Workshop 的 Source、Build、Ready、Cancel、Refresh
  工作流；UI-only Build 不启动 Node；
- Source/Release Store 的 Windows 路径冲突、特殊文件、containment、digest tamper 和
  owner/project 隔离校验。

最小证据已形成：DB ID/schema 20、MiniApp schema 6、repository 8、lifecycle 31，
Source/Release Store 3、M1 application 3、App route 3、API types 535、Agent control
plane 22、MiniApp UI 9、Agent/模型选择 UI 32，i18n、UI production build 和受影响
Rust crate check 通过。Windows 全量 `cargo fmt --all --check` 仍受文件名长度限制，
不改变定向格式检查结果。

因此下一执行切片是 `M1-0-02-B`，只先实现 UI-only 的：

1. Ready → Manual Publish 的 owner/pointer/release/catalog 原子切换；
2. Active/Previous 指针和 `active_release_epoch` 的 exact CAS；
3. Surface launch descriptor 与 Host-owned epoch/release fence 的 Desktop 入口；
4. 已授权且严格 UI-only 的 auto Publish 判定。

该切片不提前实现 dedicated Service Host、真实 MessageChannel process adapter、
Files/Private SQLite、Share/Import、永久删除或跨平台原生验证。M1-1 Service/Bridge
仍由后续切片领取，不能用现有内存 foundation 测试代替生产完成度。
## 2026-09-07 实施进展修订

1. 普通 Plugin 的 Kernel↔Node Adapter 已按 canonical `CapabilityKind` 接通 Tool、
    Context Contributor 和 Resource Provider；完整 `ResolvedCapability`、Mount 与 Artifact
    digest 参与每次执行和资源句柄缓存。`plugin-package-v1` 不发布 Role Contract、
    Role Provider 或 Package-authored Plugin Service，也不以 synthetic Role 包装普通能力。
2. Config/Credential/KV exact CAS、stable `dataDir` 与 owner mutation 已进入 migration
    069 和真实 SQLite repository；`nomifun-plugin-service` 正在接通 Artifact、Candidate、
    Apply/Restore/Uninstall/Delete-data、Host fence 与 Operation application service。
    该 service 不暴露直接 Host invoke，正式能力调用只允许进入 Kernel consumer 主链。
3. `nomifun-miniapp-platform` 当前已提前冻结新的 M1
    domain/application/runtime/data port 与 15 项内存合同测试：Host-owned Bridge 保留
    exact Active pointer 并拒绝 UI-only Files/Private DB；dedicated Service Host 已固定
    on-demand/continuous、动态全局容量、有限 crash backoff、Retry、idle reap 和 generation
    fence；Storage 已固定 owner-scoped KV checked revision/CAS、Files handle、Private DB
    窄 SQL、additive ledger 与提交前取消边界。在 migration 071+、真实 SQLite/Node
    process adapter、production composition/routes、产品入口和旧链删除完成前，这些
    foundation 不计作 M1 实施完成，也不读取或迁移旧 MiniApp 数据。
4. NomiCore 当前 Desktop 组合根已接入 Plugin application service、Artifact Store、
    Shared Host fence 与唯一 Kernel Registry publisher；enabled Mount 会在启动时按
    exact Artifact/Config/Credential/dataDir 恢复，卸载或停用会撤销 publication。
    Plugin 管理路由统一要求 installation owner 与 local-trust。Nomi consumer adapter
    完成前，Plugin capability 虽进入统一 Catalog，但对 Agent 明确 unavailable。
5. bundled JavaScript Host 资产按内容 digest 物化到应用数据目录，安装版不读取
    `CARGO_MANIFEST_DIR`。`nomifun-js-authoring` 已建立统一 ID/digest 的 Source Store、
    JS/TS scaffold、source snapshot、dependency exact-lock 与 staging cleanup；npm
    resolver/cache、fixed packer 和 Build Host 继续作为 N1-4 未完成项。
6. Plugin Project 创建已进入真实 Source Store 与 SQLite：migration 070 独立持久化
   用户可见的 `display_name/description`，既有行只做一次性回填；Source Store 持有
   canonical `dependency-lock.json`，数据库记录真实 Source/lock digest 与初始 generation。
   `plugin project delete` 已接 owner/revision/generation/Ready Candidate exact CAS，
   只删除 Project/Source/lock/Ready，不级联已安装 Mount 或运行数据。
7. `/plugins` 已成为 Desktop Plugin 工作台入口，Library/Workshop 分别承担已安装
   Plugin 与作者 Project；创建、预构建导入、Build、Candidate Test、Apply、Operation
   cancel、Project 删除和 Mount 生命周期动作均使用产品 DTO 与 HTTP bridge。MCP 页面
   只保留 MCP。Library 当前已按 exact JSON Schema 提供 primitive/enum Config 编辑，
   unsupported schema fail closed；任何 password/writeOnly/secret 配置字段都不进入
   draft、DOM 或请求，作者必须改用 Credential slot。Credential 只编辑
   `slot_key -> credential_id` reference，不新增枚举或 secret 回显。该 UI 仍以真实
   Build/Test executor、Runtime Manager 和 Desktop accessibility/视觉验收为关闭条件，
   不能因 production build 通过而提前签署 `N1-U-01`。
8. Windows 执行顺序保持不变：先关闭 N1 Plugin，再完成 M1 MiniApp，之后冻结唯一
   `RC-WIN-01` cohort；只有该 cohort 关闭后才交接 macOS arm64 与 Linux Desktop x64。
   本阶段不领取跨平台开发结论，也不运行手机视口。
9. Candidate Test 已接入独立的真实 Node Host role。它复用同一受管 Runtime、私有
   NDJSON transport、watchdog、generation fence 与整进程树回收实现，但不与生产
   Shared Extension Host 共进程。每次测试使用一次性 dataDir，默认不注入生产
   Credential；只完成真实 Artifact load/activate 而没有受管测试输入时，对带可调用
   contribution 的 Candidate 记录 `needs_test_input`，不得把“可加载”冒充“行为测试通过”。
10. `N1-3-01` 已关闭。ManagedLocal Catalog 不再从 raw manifest、Package source kind
    或命名约定反推 provenance：Kernel 的 Tool、Context、Resource、Skill 和 MCP 五类
    materialization 原样进入 Control Plane。ManagedLocal MCP-backed Capability 同时保留
    MCP binding identity、真实 Mount 和 Package Artifact，不复制第二个 MCP contribution；
    Skill 以 canonical `skill:<skill_id>` contribution lock 进入 Revision。Agent picker、
    impact、Agent Snapshot 和 non-Agent operation lock 共享 exact
    Mount/contribution/contract/Artifact 事实；duplicate/missing/cross-owner fault 在
    Registry swap 前失败并保留旧 generation。普通 Plugin Tool 已有独立 non-Agent
    operation handler/context，Gateway/UI/Automation 等 consumer 不需要伪造
    AgentSession；Agent consumer 仍必须走 Snapshot-bound 路径。当前 Kernel 26、
    Control Plane 17、Agent Platform 23、JS Adapter 4 项定向验证通过。
11. JavaScript Authoring foundation 已接入真实 Plugin Build：canonical Source
    Manifest、JS/TS/local module、exact pure-JS npm lock/cache、static bundler、受控
    `node:*` imports、Node 24 Build Host、resources、cancellation 和 immutable
    `plugin-package-v1` 均已形成可运行主链。Build 输出从与 Source/npm cache 分离的
    staging 再次进入 Artifact Store admission；Build Executor 和 operation cancel 共用
    exact operation flag，取消返回前证明 worker/process/staging 已退出。base current 和
    compatibility diff 由 application service 从 linked Mount 与 canonical Manifest
    计算，不接受 Builder 自报。Authoring 27、Plugin Service 22、真实 Build E2E 2 和
    App crate check 通过。production npm registry client、dependency lock mutation、
    Chat Dev Source 编辑、MiniApp build profile、Share/CLI 与安装版闭环仍属于未完成项，
    因此当前不关闭 `N1-4-01` 或 `N1-2-03`。
12. `plugin-package-v1` 已把 contribution schema registry 冻结为 Artifact 的正式组成：
    Tool input/output、Context 和 Event schema ref 必须精确出现在 registry 中，并以
    `schema://...#<sha256>` 绑定 canonical 内容；missing、extra、digest tamper 均拒绝。
    Packer、Artifact Store、Kernel Adapter 与 App schema resolver 使用同一 registry，
    Nomi 不从 capability ID、请求参数或最新 Catalog 猜测 schema。
13. Nomi Agent consumer 已改为 Snapshot-bound 动态 Plugin Tool Session。App 从持久化
    Conversation/Binding/Revision/Snapshot 链解析 exact Mount/Artifact，在 Runtime
    single-flight factory scope 内注册 FunctionTool action，并复用现有 deferred
    ToolSearch；UI-only、non-Agent 和 Hidden action 不注册。调用始终进入唯一 Kernel，
    provenance 或 Artifact 漂移在 Host 前 fail closed。`N1-3-02` 的实现目标已完成，
    最终真实模型/安装版 smoke 仍统一属于 `N1-V-01`。
14. Runtime Manager 后端已提供 installation-owner/local-trust 保护的 status、probe、
    official LTS download、switch begin/decision 五个产品动作，并通过 migration 071
    持久化 Runtime fingerprint、absolute executable path、validation 与 revision CAS。
    当前只完成 Foundation Host Hello/cleanup；Plugin Shared Host、Candidate Test、
    Build Foundation 与 MiniApp Service 尚未由同一个 committed Runtime authority
    协调，所以生产试切换明确返回 `JAVASCRIPT_RUNTIME_NOT_COVERED`。在统一
    drain/stop/validate/commit-or-restore coordinator 与 Desktop Runtime Manager UI
    完成前，不得把该 foundation 记作 Runtime 产品闭环。
15. 2026-09-07 的 Runtime authority 收口已把 §4.3C 的关键边界落实到代码：
    `RuntimeAuthority` 是 Plugin Shared/Candidate/Build Host 的唯一 committed binding；
    所有 Runtime-backed demand 通过 read lease admission，切换通过全局 write fence。
    Selection mutation 采用 `pending → validation → commit/abort` 三段 CAS，coordinator
    作为 detached task 运行，HTTP 请求取消不会遗留一个已释放 fence 的 durable pending。
    Candidate validation 返回真实 Foundation Hello/process-tree proof、固定 Build Host
    最小编译结果与每个 enabled Plugin Mount 冷加载结果；Mount 使用一次性 dataDir 且
    不注入生产 Credential。MiniApp production Service 尚未接入时明确返回 `NotCovered`，
    交给用户裁决，不伪造全通过。旧 068 fingerprint 没有 executable path 证据时由
    071 清空并要求重新 probe。
16. Runtime-bound Plugin 组合已删除启动时独立 PATH Node 捕获：Shared Host、Candidate
    Test、Build Host 在 exact committed Runtime 下懒创建；切换前释放 Kernel 全部
    Runtime-bound resource handles，切换后按新 binding 重新 acquire。Plugin Enable 在
    无 Runtime 时于 durable mutation 前拒绝，符合 §4.3B 的 disabled/needs-runtime
    语义。`/settings/execution-engines` 已物理改为 Runtime Manager，旧本机 Agent
    检测卡片与 Agent runtime alias 页面已删除。
17. Runtime coordinator 的最终产品关闭条件仍未满足：MiniApp Service participant、
    installed-app Build→Test→Apply/Restore、Windows fault injection、Desktop
    accessibility/视觉验收和全量 UI typecheck 尚未完成。当前实现可以安全地继续开发，
    但不能把 `N1-1-01`、`N1-2-03` 或 `N1-U-01` 标为 `closed`。
18. M1-0-01 已形成一批不读取旧链的 SQLite foundation：migration 072 和
    `nomifun-db` repository 只负责 owner-scoped Product/Project/Artifact/Release
    持久化及 CAS。它使用当前数据库的 logical-reference/无物理 FK/trigger 合同，
    不迁移、双写或 alias 旧 `miniapps`；生产路由、MiniApp runtime、Bridge、Storage
    与 UI 仍按 §6 依赖顺序后续实施。

## 2026-09-07 M1-0-02-A 实施切片冻结

本节只冻结本轮 `M1-0-02-A：UI-only Source → Build → Ready` 的实现填充，不扩大
§4.4～§4.5 已确认的产品合同，也不提前实现 Service、Bridge、Publish 或 Rollback。

### A. Source Store

- MiniApp 使用独立于 `nomifun-js-authoring` Plugin Source Store 的
  owner/project Source Store；顶层身份必须保持 `miniapp_id + project_id`，不能通过
  Package ID 或旧 `miniapps` 表推导。
- UI-only Source 的受管文件集合只允许 `index.html`、`ui/**`、固定的项目元数据和
  exact dependency lock；实现层将入口规范化为 `ui/index.html`，并以 canonical
  snapshot digest 标识整棵 Source。
- Source 写入先进入 owner/project 隔离的 staging，再以原子替换更新 Source head；
  路径 containment、Windows 大小写/NFC 冲突、空文件、特殊文件和取消清理必须在
  Store 边界拒绝。Source Store 不接受 `service/main.mjs`，也不从 Plugin Store 复制
  文件。
- Project source revision、source digest、dependency lock digest、build profile version
  与 build generation 在同一 owner CAS 语义下更新；Source 改变不会隐式修改 Ready、
  Active 或 Previous。

### B. UI-only Build 与 Operation

- Build application service 只接受当前 owner/project 的 exact Source head，并在开始时
  捕获不可变 source/lock snapshot；同一 MiniApp 同时最多一个 active Build。
- Build 使用固定 `miniapp-release-v1` UI-only profile。它可以复用既有 canonical
  manifest、路径校验和 digest 规则，但不能把 Plugin `plugin-package-v1` 顶层身份、
  Source Store 或 Package lifecycle 当作 MiniApp 实现。
- UI-only Build 不启动 Node；它只把已验证的静态 UI bytes 送入固定 builder 和
  Release Store。任何自定义 build/install/postinstall script、Service source、native
  addon、dynamic import 或未锁定依赖都 fail closed。
- Build 持久化一个最小 `Build` Operation：开始时写 `running`，在成功写入 Ready
  后写 `succeeded`；失败写 `failed`；取消只在 Ready 提交前生效，跨过提交边界后返回
  conflict，不事后把成功 Ready 改成 canceled。Operation 不扩展成通用 phase engine。
- staging、Build log 尾部和 Operation 都以 owner/project/build generation 关联；App
  crash 或取消后必须能清理 staging，不能留下可被下一次 Build 误认的半成品。

### C. Immutable Release 与 Ready CAS

- Release Store 保存完整 manifest、每个规范化文件的 bytes、逐文件 digest、manifest
  digest 和 canonical Release digest；文件 bytes 与 manifest 不一致时不得创建 Release
  row。
- Release artifact 使用独立于 Plugin Artifact Store 的 MiniApp 端口，但可以复用同一
  content-addressed CAS、containment 和 published inventory 原则；不能让 MiniApp
  Release 伪装成 Plugin Mount 或 Capability Package。
- Build 成功时按 `project source digest + dependency lock digest + build profile
  version + build generation` 写入一条 managed Release，并通过 exact Product/Project/
  pointer CAS 原子替换旧 Ready。既有 Active/Previous 不在该切片中被切换。
- Build 失败、取消或 digest admission 失败时，旧 Ready、Active、Previous 和 Catalog
  保持不变；成功但后续 DTO 组装失败也必须返回明确错误，不允许返回一个看似 Ready
  的半状态。

### D. App/UI 边界

- 新增的 MiniApp Build 路由只接受 installation owner 与 local-trust 请求，并把
  revision、source digest、build generation 和 operation identity 交给 application
  service 做最终校验；UI 不得自报 Ready、Release、owner 或 digest。
- Library/Workshop 本轮只展示 Source 状态、Build 状态、Ready digest/时间和可操作的
  Build/Cancel；不显示不存在的 Publish/Surface/Service 控件，也不恢复旧 Guid/
  Conversation MiniApp 入口。
- 本轮的最小验收必须同时覆盖 fresh owner 隔离、Source CAS、Build success/failure/
  cancel、file/manifest digest、一致的 Ready lineage、staging cleanup 以及
  UI-only Node process=0。Windows Desktop x64 是唯一当前执行环境；跨平台验证在
  Windows cohort 冻结后统一交接。

## 0. 怎样阅读这份文档

这份计划先回答产品问题，再给出足够实施的技术边界。它不尝试提前设计所有异常、权限、兼容和远期扩展。

文中只有四种状态：

- **保留确认：**用户已经明确确认，且通过本轮简单性审计，继续作为设计前提；
- **User confirmed：**本轮逐组验收已经明确确认的 Plugin/MiniApp 产品与架构合同；七组已形成设计基线，AgentPreset AP 门禁另由 05 §15 管理；
- **实现填充：**方向已确定，但字段名、默认值、版本号等应由实现切片冻结；
- **后置：**首版不做，也不预埋占位状态和隐藏入口。

本文的七组 Plugin/MiniApp 产品与架构合同作为设计基线保留；它们不等于 AgentPreset 前置门禁已完成。当本文与旧文档、README 或 DECISIONS 中针对 Phase N 的前瞻性描述冲突时，本文直接优先；Stable v2 已冻结的事实不受影响。涉及 AgentPreset 的产品、API、Revision、Snapshot、Binding 和旧模型删除时，05 §15 优先。

进入本文的 N1-0 代码实施前，必须先核验 05 §15 的 AP-0～AP-7：统一 Agent 工作台已经成立，平台 Capability Catalog 已能服务至少一个 Agent 和一个非 Agent 消费者，旧 `/api/presets` 生产可达性为 0，且 ContributionLock/impact diff 已有代表性测试。未满足时，06 中的 “Catalog、Agent” 只表示待接入的消费者适配，不表示可以先行开发 Agent 专属插件链。

## 1. 产品结论

### 1.1 二期实际交付什么

二期不是一套笼统的“Agent 插件系统”，而是两个并列的平台产品和一套共享技术底座：

| 产品 | 用户理解 | 首版价值 | 是否有独立 UI |
|---|---|---|---|
| Plugin | 扩展 NomiFun 平台能力的组件 | 向平台 Capability Catalog 贡献能力，供 Agent、Gateway/Remote、UI/业务域、Automation、MiniApp/Service 等兼容消费者选择 | 否，只提供配置和诊断页面 |
| MiniApp | 用户可以打开、使用和持续运行的应用 | 提供自己的 UI，并可带一个全栈 Service | 是，拥有自己的 Surface |

Agent 工作台不计为第三个 Plugin/MiniApp 产品；它是由 05 §15 定义的 Agent consumer surface，负责选择和使用平台能力，不拥有 Package、Release、Service 或 Capability。

**保留确认：**MiniApp 与 Plugin 不合并为同一种东西。二者共享 JavaScript Host Foundation、Runtime Manager、Service Host 和公共协议基础设施，但拥有不同的顶层身份、Manifest、产品入口、发布生命周期和删除语义。

交付顺序固定为：

1. 先完成 Phase N1 Plugin 的 Windows 功能与候选 Gate；
2. 复用已冻结的 Node、Host、Package、SDK 和 Authoring Foundation 完成 MiniApp M1
   的 Windows 功能与候选 Gate；
3. Windows Plugin/MiniApp 全部完成后冻结同一最终 cohort，再统一执行 macOS arm64
   与 Linux Desktop x64 required 原生验证；
4. Marketplace、第二 Runtime、远程分发等后续能力不反向阻塞首版。

默认的一站式零环境开发入口只有 Chat Dev：用户下载安装 NomiFun 后选择“创建 Plugin”或“创建 MiniApp”，由 Agent 作为 Project-scoped authoring client 使用受限工具完成编辑、依赖解析、Build、Test、上线和导出；接收方把带 Source 的 NomiFun Share Bundle 导入即可使用或继续开发。外部 IDE/AI/工具链按公开合同生成并导入 prebuilt Artifact 是另一条正式支持的开发交付路径，但 NomiFun 不负责复现其外部构建环境。首次缺少 Node 时，Runtime Manager 按既有决策显示一次官方 LTS 下载确认并自动完成获取、校验和配置；Chat Dev 用户不需要自行准备命令行开发环境。

### 1.2 JavaScript、TypeScript 和 Node 的关系

- JavaScript 与 TypeScript 都是一等开发语言；
- Node.js 是首版唯一正式执行 Runtime；
- Node 执行 JavaScript，不直接执行 TypeScript；
- Plugin 与 MiniApp 的默认完整开发入口是 NomiFun Chat Dev Mode：用户/Agent 在受管 Project 中完成 Source 编辑、依赖、Build、Test、上线、Export 和 Import，不要求安装外部 IDE、npm、TypeScript、packer 或 SDK；同时保留外部 IDE/AI/工具链产出符合 NomiFun 合同的 prebuilt Plugin/MiniApp Artifact并导入使用；
- Plugin SDK runtime 和首版正式支持的 pure-JS npm 依赖由 NomiFun Build Profile 或外部等价构建打入最终 `main.mjs`；无论来源，Runtime 都只安装和执行通过同一 validator 的 immutable Artifact；
- 首版只有一个正式可执行入口 `main.mjs`，source map 仅作可选诊断资源；
- NomiFun 的测试基线固定为一个社区官方 Node LTS，不维护多版本测试矩阵。

消费侧不强迫用户只能使用测试基线。满足最低兼容条件的已有 Node 可以直接使用；偏离推荐版本时首次给出一次非阻断提示，之后只保留信息状态。

### 1.3 Plugin 能扩展到哪里

Phase N1 Plugin 可以向平台贡献：

- Tool；
- Context Contributor；
- Resource Provider；
- Skill；
- MCP-backed Capability。

这些贡献先进入平台 Capability Catalog，再由声明兼容的消费者使用。Agent 工作台、Gateway/Remote、Automation、UI/业务域或 MiniApp Service 可以是消费者，但没有任何一个消费者拥有这些贡献。每项贡献都有 Package/Mount provenance，不能伪装成 NomiFun 内建能力，也不能因为支持 Agent 就复制成一套 Agent 专属目录。

Contribution/Manifest 必须区分：

- 能力身份、合同版本与 `contract_digest`；
- owning Package、Mount/Release 和 Artifact provenance；
- 支持的消费者或 Surface；
- Tool、Context、Resource、Skill、MCP-backed 等 typed contribution；
- Runtime feature、typed resource 和 availability。

`supported_surfaces` 或等价字段只描述可被哪些消费者理解，不授予权限，也不代表 AgentPreset 拥有 Capability。Agent 工作台只筛选 `agent` 兼容且已经正式物化的条目；其他消费者按自己的 resolver 和 operation lock 使用同一 Catalog。

Catalog 的物化状态与 consumer/surface availability 必须分开记录：某项贡献可以已发布且可供 UI 或 Gateway 使用，但对 Agent 因资源、合同或 Runtime 原因不可用；不能用全局布尔值覆盖这种差异。

某项 Plugin 或 MiniApp contribution 可以只服务 UI、Gateway、Remote、Automation 或其他业务域，而完全不对 `agent` Surface 开放；“能被 Agent 选择”是可选消费场景，不是 Plugin/MiniApp 的存在前提。

NomiFun-authored Plugin 可以有平台提供的 Project/Source/Build/Candidate Workshop；它是 NomiFun 的作者工具，不是 Plugin 自己贡献的 UI，也不改变 Plugin“无独立产品 Surface”的身份。

Phase N1 Plugin 不可以：

- 替换 NomiFun 主窗口、导航、Workbench 或全局布局；
- 覆盖内建 ID、接管 Kernel、Runtime、Agent 执行循环或 IDMM 核心实现；
- 修改其他 Plugin 的受管配置、KV 或 Credential binding；
- 向 NomiFun 核心数据库注册 Migration；
- 提供长期后台 Service。

Plugin 可以贡献一个供用户选择的替代能力，但不能透明覆盖已有核心能力。MiniApp 可以完全定义自己的应用内 UI，但也不能替换 NomiFun Shell。

### 1.4 AgentPreset 集成只是一个消费者适配

本文不重新定义 AgentPreset。Agent 的唯一产品入口、公共路由 `/agent`、四种模式、Revision payload、ContributionLock、Snapshot、Binding、Session 和旧 `/api/presets` clean cut 全部以 05 §15 为准。`/presets`、`/settings/agent-presets` 和 `/settings/agent` 不是新的产品入口，只能作为有期限的一次性迁移跳转；`/settings/execution-engines` 只保留 Runtime Manager。

Plugin/MiniApp 与 AgentPreset 的关系固定为：

```text
Plugin current / MiniApp Active Release
                → 发布平台 Contribution
                → Materialize 到 Capability Catalog
                → Agent 工作台选择兼容 Capability/Skill
                → AgentPreset Compiler 生成 ContributionLock
                → Snapshot 记录 exact target/release provenance
```

面向消费者的 Capability/Skill/Tool/Context/Resource contribution 才进入可查询的 Platform Capability Catalog；仅作为 canonical façade 内部实现的 Role Provider 或 Host-only contribution 走共享 Materializer/Registry 内部索引，不出现在 Agent picker，也不形成第二个 Agent capability identity。

禁止出现：

- `contributes.presets`、`ExtPreset`、`ResolvedPreset`、`/api/presets` 或 `/api/extensions/presets`；
- Package-owned `AgentPresetSource::Package` / `agent_preset_templates.source_kind=package` 分支；
- Plugin 安装后自动创建、修改或扩张 AgentPreset；
- “全面模式”自动纳入全部 Plugin/MiniApp；
- Agent 直接选择 Package、读取 Plugin 私有 `dataDir`、改变 Plugin/MiniApp 发布授权；
- 为 Agent 单独复制 Package lifecycle、Catalog 或 Runtime Host。

如果未来允许 Package 提供 `agent_template`，它只能作为创建 Agent 的可审阅种子，不是第二种 Agent 类型，不自动授予能力，也不属于 Phase N1 的前置交付。

### 1.5 “可信代码”到底意味着什么

安装和启用即表示用户把 Plugin 或 MiniApp Service 当作本机可信代码。首版不建设 sandbox、权限清单、代码签名审批、WASI、逐 API 拦截或安全评分。

仍保留五项基础正确性边界：

1. Package staging、路径 containment、digest 和原子发布；
2. NomiFun 核心数据库、Credential 和 owner namespace 不通过正式 API 越界；
3. 共享 Host 故障时能够终止整棵 Node 进程树；
4. MiniApp Release 与 Catalog 原子切换，不能出现半新半旧；
5. 首发三个 required cells 必须用真实制品验证；optional cell 只有在实际宣称支持和交付时，才必须关闭自己的真实制品 Gate。

由于没有 sandbox，恶意代码仍可能通过原生文件系统或外部命令绕过产品 API。首版明确依靠信任，而不是用无法兑现的权限 UI 制造安全感。

## 2. 首版范围和明确非范围

### 2.1 Phase N1 Plugin 必须完成

- 先通过 05 §15 的 AP-0～AP-7 admission gate（门禁可使用现有 first-party Capability，不等待 N1 Plugin/MiniApp 代码）；N1 不补写第二套 AgentPreset、旧 Preset 兼容层或 Agent 专属 Capability Catalog；
- 从本地目录或压缩包安装 prebuilt Package；
- Package 校验、managed root、启用、停用、显式替换和卸载；
- schema 配置、Credential slot、KV/CAS 和稳定 `dataDir`；
- NomiFun 内置 JavaScript/TypeScript SDK/types、Project scaffold、fixed Build Profile 和 conformance runner；不要求用户准备外部开发环境；
- NomiFun Plugin Project、Managed Source、单 Ready Candidate、Candidate Test 和 Candidate/current 影响 diff；
- 默认 Ask Before Apply；用户可按 Plugin 授权本地 authored Candidate 在 shared Host 空闲时执行 exact-contract-compatible auto-Replace；Breaking 或无法证明兼容的变化始终人工；
- Plugin Share Bundle/Prebuilt Artifact 的 Export/Import：NomiFun Bundle 可携带 Source 供继续开发，runtime-only 外部 Artifact 可导入使用但不伪造 Source；用户数据和 Credential 默认不进入分享包；
- 共享 Extension Host、私有 IPC、watchdog、整 Host 回收；
- 平台 Capability Catalog、通用 provenance/availability、至少一个非 Agent consumer adapter，以及对 05 §15 已冻结 Agent 工作台/Revision/Snapshot 主链的消费者接入；
- Desktop 管理页面与本地 Headless CLI；独立 Linux x64 Headless 制品不作为首发阻断项；
- CSV/JSON Tool 参考插件，以及独立的故障测试 fixture；
- 首发 required 平台按 `Windows x64 → macOS arm64 → Linux x64 Desktop` 依次交付；macOS x64 与 Linux x64 Headless 降为最低优先、可后续追加的 optional cells。

### 2.2 MiniApp M1 必须完成

- 全新数据模型，不迁移、读取或自动删除旧 MiniApp 数据；
- UI-only 与 UI + Service 两种形态；
- 一个 MiniApp 最多一个 `main` Service；
- Source/Build/immutable Release、一个 Ready、一个 Active、一个 Previous；
- 默认人工 Publish；用户可按 MiniApp 授权严格纯 UI 自动 Publish；显式 Rollback、Enable、Disable、Trash 和永久删除；
- Host MessageChannel Bridge，不开放 localhost 端口；
- UI/Service 共用受管 KV，Service 可选受管 Files 与 Private SQLite；
- `on_demand` 与 `continuous` 两种 Service 生命周期；
- Capability 随 Active Release 原子进入 Catalog；
- Export、Import-as-new 和首发 required 三平台验证；optional cells 在实际追加交付时再关闭各自 Gate。
- 默认无用户数据的 MiniApp Share Bundle 与外部 prebuilt Artifact Import；携带 Source 时可继续开发，只有 Runtime Artifact 时可使用但不伪造 Source；携带业务数据的 disabled Whole-App Backup 保持独立入口。

### 2.3 首版后置

- Marketplace、publisher、在线发现、URL/Git 安装和远程 Registry 自动更新；本地 NomiFun-authored Plugin 的用户授权 auto-Replace 属于 §4.7，不属于该排除项；
- 为 VS Code/终端/第三方 IDE 复制一套与 Chat Dev 等量的环境安装、依赖编排和多工具链兼容产品；外部开发产物导入本身属于正式范围，平台必须发布 Artifact Schema/SDK types/validator 合同，但不保证复现每一种外部 build tool；
- Bun Runtime provider、第二 SDK、Runtime 插件市场和多 Runtime 同时运行；
- Browser Engine 与 Embedded Surface 选型；本文不锚定 Electron、WebView 或任何一体化桌面方案，也不把该选型设为 MiniApp M1 admission blocker。M1 复用届时已有的 Surface 承载能力，未来替换 Engine 时另立 ADR；
- 插件依赖 DAG、跨插件命令总线、Plugin Service；
- Plugin authored UI、Shell replacement、同 Capability ID 抢占或静默劫持；显式 Role Binding 替换仍按 05 的系统能力合同处理；
- 自动 Promote、60 秒观察窗口、自动 Rollback；
- MiniApp in-place Whole-App Restore；
- Active MiniApp 在线快照、数据库时间回滚、增量备份；
- 通用权限系统、sandbox、签名链和审批；
- 多 Service、Service pool、优先级抢占和持久化容量租约；
- 通用 Transition/Operation 工作流引擎；
- hot reload、运行中 Plugin 自改 managed target、`replaceSelf()` 和 Plugin 自行开启部署授权。

后置意味着首版 Schema、IPC、UI 和测试中均不为这些能力预埋不可达状态。

本节中的 **Plugin Service** 专指“由 Plugin Package 自带并长期运行的服务能力”，不指
NomiFun Rust 侧负责 Library/Workshop/生命周期编排的
`nomifun-plugin-service` application service。后者属于 N1 产品主链，前者保持后置。

## 3. 最小共享架构

```text
NomiFun Rust Host
  ├─ Package / Product Manager
  ├─ JavaScript Authoring & Build Foundation
  │    ├─ plugin-package-v1
  │    └─ miniapp-release-v1
  ├─ Platform Capability Catalog
  │    ├─ Agent consumer adapter（复用 05 §15）
  │    ├─ Gateway / Remote / Automation adapters
  │    └─ UI / 业务域 / MiniApp Service adapters
  ├─ Runtime Manager（首版只有 Node resolver）
  ├─ Runtime Supervisor
  │    ├─ shared Extension Host
  │    │    └─ ordinary Plugin Mount A / B / ...
  │    ├─ transient Build Host
  │    └─ dedicated Node Service Host
  │         └─ one active MiniApp Service Mount
  ├─ Host-managed Config / KV / Credential / MiniApp DB
  └─ MiniApp Surface Host + MessageChannel Bridge
```

共享原则：

- 普通 Plugin 共享一个按需求启动的 Extension Host；
- Capability Catalog 是平台级供给目录；Agent integration 只是一个 consumer adapter，不能决定 Plugin/MiniApp 的产品身份、owner 或生命周期；
- NomiFun-authored Plugin 与 MiniApp 共享 Source/npm/lock/cache/packer/Build Operation 基础设施，但使用不同 Build Profile 和输出合同；Build 只在显式 Project-scoped authoring request（可由 Agent 发起）期间使用 transient Build Host；
- 每个 active MiniApp Service Mount 独占一个 Node Service Host；
- UI-only MiniApp 不启动 Node；
- Plugin 没有长期后台生命周期，只有能力请求才激活；
- MiniApp Service 可以 `on_demand` 或 `continuous`；
- Extension Host 与 Service Host 共用 Node 启动、IPC、日志、watchdog 和进程树回收基础设施，但使用不同 role protocol 和 SDK；
- MiniApp 与 Plugin 不共享顶层 Manifest，也不能互相伪装身份。

共享 Extension Host 是显式共同故障域：一个 Plugin 导致进程 crash、死循环升级或 IPC 断开时，Supervisor 回收整棵进程树，并失败当前 generation 内全部 in-flight/queued 调用；Host 保持 stopped，下一次新 demand 或用户 Retry 才创建干净 generation。不会尝试在同一个 Node 进程里“只硬回收一个 Plugin”。Plugin 的正常 dispose 只做协作式清理，未注册资源允许存活到当前 Host generation 结束。

## 4. 七组简化实施合同

以下七组是 Plugin/MiniApp 自身的产品与架构合同，已形成设计基线；它们不包含 05 §15 的 AgentPreset 前置建设。只有 AP-0～AP-7 全部通过后，下一步才可以进入 N1-0 机器合同冻结和实现填充。不再恢复旧文档中的几十个 P/B 编号，也不把字段级实现填充重新扩大为产品 A/B 裁决。

### 4.1 Plugin 数据与 Credential

#### 4.1A Plugin 文件与 SQLite（User confirmed）

**已确认方案一：**Phase N1 受管存储只保留 Config、KV/CAS、Credential slot 和一个稳定 `context.storage.dataDir`；撤销旧稿中的 Host-managed PluginFiles，首版也不建设 Plugin Database、统一 quota 或 backup。

Plugin 可以直接使用原生 `node:fs`、`node:sqlite` 和 pure-JS npm 数据库封装管理自己的 `dataDir`，包括自行建表、执行 SQL 和 Migration。NomiFun 不提供 PluginFiles pseudo-VFS、不返回核心 SQLite 路径，也不把 Plugin Migration 注册进核心数据库。

```text
managed config / KV / credential binding
              +
stable absolute dataDir
              ↓
Plugin uses raw node:fs / node:sqlite by itself
```

产品含义：

- 普通开发者获得完整 Node 数据能力，不受一套不完整 Host 文件 API 限制；
- NomiFun 只备份、计量和解释受管 KV；`dataDir` 内数据库、文件、server、timer 和外部命令副作用由 Plugin 自己负责；
- 未来只有真实需求证明需要统一 backup/quota/Migration 产品体验时，才独立立项 Host-managed Plugin DB。

#### 4.1B Credential 解析与轮换（User confirmed）

**已确认方案一：**Host-side `resolve(slot)` 每次返回当前 Credential；轮换、解绑或删除在下一次 resolve 生效，Plugin 自己缓存的旧明文不强制撤回。需要立即清除缓存时由用户显式重建整个 JavaScript Host，不要求重启 NomiFun，也不建设 secret revision/history/snapshot 状态机。

Credential 最小合同：

- Manifest 声明 owner-scoped `secret_text` slot；
- 持久化 binding 只有 `slot_key -> credential_id`；
- trusted Plugin 在主线程每次通过 Host-side resolve 得到当前明文；
- 不提供 Credential 枚举、跨 owner resolve 或历史 secret revision；
- Credential 更新后，下一次 resolve 返回新值；Plugin 自己缓存的旧明文无法被 Host 强制撤回；
- UI 可提供“立即重建 JavaScript Host”动作，强制所有 resident Plugin 重新初始化，但不要求重启整个 NomiFun；
- 日志和诊断继续做基础 secret redaction。

实现验收只需证明：namespace 不串写、binding 不越 owner、secret 不进入持久化 Plugin 配置或日志、`dataDir` 在替换后稳定。

### 4.2 Package、Replace 与 Uninstall

**保留确认：**一个 Package ID 只有一个 active Mount；一个 Package 只有一个声明式可执行入口 `main.mjs`。

#### 4.2A 本地 Package Version 与 Digest（User confirmed）

**已确认方案一并经 §4.7 扩展：**本地 Package 的 Version 是作者标签，canonical digest 是精确内容身份。同一 `package_id + version` 的不同 digest 只能通过用户针对本次 Replace 的显式确认，或针对 linked authored Plugin 预先授予的 `auto_compatible_when_idle` standing authorization 应用；managed target 仍不可原地修改，普通 Install 不静默替换，不建设永久 `PackageVersionClaim`。未来 Marketplace/Registry 可以对公开发布坐标独立执行永久 version→digest 绑定。

最小 Manifest 只表达：

- Package ID、Version 和 host contract version；
- 支持的 OS/arch；
- `main.mjs` digest 与整包 digest；
- contribution 声明；
- 每项 contribution 支持的 consumer surface、typed resource/runtime contract 和 availability 约束；
- config schema 与 Credential slots；
- 必要的名称、描述和 provenance。

整个 Package tree 经过规范化后计算 canonical digest。SDK runtime 和正式支持的 pure-JS npm 依赖必须已经进入 `main.mjs`；Package Installer/Runtime 不安装 npm 依赖、不运行 build script，也不把 source map 缺失当成安装失败。Chat Dev 的 Authoring Foundation 在 Artifact 形成之前按 §4.7 解析依赖和固定构建，但同样不运行用户自定义 script。Native addon、postinstall 和运行时 `node_modules` 不属于首版兼容性承诺；即使 trusted code 把它们作为不透明资源自行加载，ABI、OS/arch 与可移植性也由开发者负责。

安装流程只有一条：

```text
copy to staging
→ archive/path containment check
→ digest + Manifest + platform check
→ create immutable Ready Package Candidate
→ Test when requested/required by auto eligibility
→ user Manual Install/Replace or authorized authored auto-Apply
→ atomic managed target + current/previous inventory update
```

`plugin install` 可以保留为首次导入后的便捷动作，但内部仍必须创建并验证同一个 Candidate，再由用户明确提交首次 Install；它不是绕过 Candidate/diff/Test记录的第二条 loader。外部 Artifact、Share Bundle 和 Chat Dev Build 都进入同一 admission。没有兼容 Node 时，Candidate 可以导入并由用户安装为 `disabled/needs-runtime`；此时无法形成本机 passed Test，也不能 auto-Apply。

- 当前平台不受 Package 支持时拒绝安装；
- Node 尚未就绪时允许完成静态安装并显示 `disabled/needs-runtime`；不自动下载或启用，只阻塞 enable、Host boot 和 invoke；
- 同一 Package ID + Version 出现不同 digest 时，普通 Install 不能继续；只有显式 Replace，或 §4.7 已授权且 exact eligible 的本地 authored Candidate 才能应用。不建设永久 Version Claim 表。

#### 4.2B Replace 失败与 Restore（User confirmed）

**已确认方案一：**Replace 只保留一代 previous target。静态检查失败时原 current 不变；新 target 成为 current 后，如果 lazy activation 失败，则保留新 target 为 current/error，不自动回退或重放原调用。用户可以显式 Retry current 或 Restore previous；Restore 只回退代码，不回退任何配置、凭据、数据或外部副作用。

Replace 只保留：

```text
current_target
previous_target?  # one-step code rollback
last_error?
```

NomiFun-authored Plugin 可以另有一个非运行 `ready_candidate?`；它属于 Plugin Project 作者状态，不是第三个 active/recovery target，不进入 Extension Host，也不改变 current/previous 的运行语义。

- staging 和静态 preflight 失败：继续使用 `current_target`；N1 不为 Replace 启动一次性 candidate Host；
- 静态检查通过后原子切换 target：新 target 成为 current，旧 target 成为 previous，并作废旧共享 Host generation；是否立即重建仍由真实 demand 决定；
- 新 target 首次 activation 失败：current 标记为 error，不自动循环激活；用户可显式 Retry current 或 Restore previous；首次安装没有 previous 时只提供 Retry、重新安装或卸载；
- Retry 不改变 Package pointer，只按 demand 创建新的 Host generation；失败的原业务调用永远不自动重放；
- Restore 只回退代码，不回退 Config、Credential、KV、`dataDir` 或外部副作用；
- 不建设 LKG 资格、历史 tuple、PackageVersionClaim 或多代 recovery graph。

任何 resident Mount 的 `current_target`、Enable 或安装状态变化都会作废并结束当前共享 Host generation：先停止接收新调用、有限等待现有调用，然后终止整棵 Node 进程树。未 resident Mount 的 Replace、Restore、Disable 或 Uninstall 只更新静态 inventory，不打断正在运行的 peer。Host 是否在故障后自动重建由 §4.3E 决定；不为单个 Plugin 建硬回收或进程隔离状态机。

#### 4.2C Uninstall、保留数据与重新安装（User confirmed）

**已确认方案一：**Uninstall 默认永久保留 non-executable Mount、Config、KV、Credential bindings 和 `dataDir`；authored Plugin 还保留关联 Plugin Project Source、dependency lock 与未应用 Ready Candidate。重新安装相同 Package ID 时不得静默复用；Desktop 必须提示并由用户明确选择 Restore，Headless 必须使用 `--restore-data`。Restore 复用原 `mount_id`，但不保证新代码兼容旧数据；fresh install 必须先通过独立删除运行数据动作清除 retained Mount data，安装过程本身不隐式删除。Plugin Project 是独立作者资产，不随 `delete-data` 静默删除。

Uninstall 默认保留 non-executable retained Mount、Config、KV、Credential bindings、`dataDir` 和可选 authored Project，且不自动过期；可执行 target 不再被 inventory 引用。重新安装同一 Package ID 时，Desktop 明确提示是否复用 retained Mount 并默认推荐复用原稳定 `mount_id`，Headless CLI 通过 `plugin install --restore-data` 显式选择。若要 fresh install，先永久删除 retained data。

用户明确选择“删除运行数据”时只写一个 `delete_pending`；Rust Host 先确保该 owner 已不在 resident Host generation 中，再在不启动 Plugin Host 的前提下幂等删除 Config、KV、Credential bindings 和 `dataDir`，并把仍存在的 Plugin Project 与被删除 Mount解除关联、使 Candidate重新要求 base/diff。删除 binding 不删除用户 Credential Store 中可能被其他产品使用的原始 secret；不存在的数据视为删除成功。删除 Plugin Project Source/Candidate 必须使用独立 `plugin project delete` 并再次明确确认；组合“删除运行数据与项目”也必须在确认页逐项列出。

#### 4.2D-1 Agent 作为一个消费者的 Package 绑定（User confirmed）

**已确认方案一：**AgentPreset Revision 依照 05 §15 生成稳定 `mount_id + contribution_id + contract_digest` 的 ContributionLock，调用时只解析当前 active 且合同完全一致的 target，并在 Snapshot 记录实际 Package version/digest。Compatible Replace 可以继续服务旧 Revision；Restore retained Mount 后可继续解析原 Revision；Permanent Delete 后的 fresh Mount 不得静默重绑。不建设历史 target 保活或同 Package 多版本并行运行。

#### 4.2D-2 Incompatible Replace 的影响处理（User confirmed）

**已确认方案一：**当新 target 删除既有 contribution 或改变其 exact `contract_digest` 时，Replace preflight 必须派生并显示所有正式消费者的影响，包括 Agent Revision/Binding、Automation、Gateway/Remote operation binding 和其他可索引 consumer lock。Desktop 由用户显式确认 Breaking Replace，Headless 必须使用 `--allow-breaking`；确认后允许切换 current target，但受影响旧锁只返回 typed contract mismatch，不自动修改或调用新合同。Restore previous 后旧合同可重新使用；不建设 Candidate Catalog、多版本执行或自动 Revision/consumer migration。

#### 4.2D-3 非 Agent 消费者的 exact lock

Gateway、Remote、Automation、UI/业务域和 MiniApp Service 不为使用 Plugin 而创建 AgentPreset。它们通过同一 Capability Catalog 和 contribution resolver 生成 consumer-specific exact operation lock，并记录稳定来源、`contribution_id`、`contract_digest` 和实际 target digest。

AgentPreset 的 ContributionLock 与非 Agent OperationLock 共享来源和合同语义，但不是同一个产品对象。Replace impact 必须覆盖两者；不得因为当前 UI 只展示 Agent 就漏报其他消费者。

### 4.3 Runtime Manager 与 Node Host

**保留确认：**Node.js 是首版唯一正式执行 Runtime；测试基线固定为一个社区官方 Node LTS；兼容外部 Node 可以复用；下载前必须由用户确认；`RuntimeManager` 保留通用产品边界，但首版不建设 provider 市场。

#### 4.3A Runtime 自动发现范围（User confirmed）

**已确认方案一：**Phase N1 只自动发现已保存的手工绝对路径、NomiFun 进程当前 PATH 中的 Node，以及已下载的官方 Managed Node。nvm、Volta、asdf、fnm 当前激活且进入 PATH 的 Node 自动可用，其他版本由用户浏览指定；不编写版本管理器专用 scanner，也不主动建立 Bun inventory，手工选中 Bun 时只提示当前 Node Profile 不兼容。

首版候选只有：

1. 用户明确指定的绝对路径；
2. 当前 `PATH` 中的 Node；
3. 用户确认后由 NomiFun 从 Node 社区官方渠道下载的 Managed Node。

nvm、Volta、asdf、fnm 等当前激活的 Node 会自然出现在 `PATH`；其他版本由用户指定绝对路径，不开发各版本管理器 scanner。首版不主动枚举 Bun；用户误选 Bun 路径时 probe 返回“不是兼容 Node Runtime”。未来可以增加 Bun provider，但不改变 Plugin/MiniApp 产品合同。

Managed Node 的准确含义是：

- 下载社区官方 Node LTS 原始发行物；
- 校验官方元数据和 digest 后放入 NomiFun managed runtime directory；
- 不二次编译、不改二进制、不换成 `nomifun-node-*` 品牌；
- 下载前必须提示，得到用户确认后才开始；
- 不建设独立于 NomiFun 的 Runtime 自动更新通道。

候选排序：用户明确选择 > 已保存的有效选择 > 与测试基线最接近的兼容 PATH Node > Managed Node。外部 Node 满足最低兼容条件但不在推荐区间时直接使用，首次提示一次，之后只保留信息状态。

#### 4.3B Runtime 未就绪时是否允许安装 Package（User confirmed）

**已确认方案一：**Package 的 staging、containment、digest、Manifest、host contract 和当前平台校验不依赖 Node；没有可用 Runtime 时仍允许安装为 `disabled/needs-runtime`，但不能 enable、启动 Host 或调用。不自动下载 Node、不自动 Enable，也不发布可调用 Contribution；Runtime 就绪后由用户显式 Enable。当前平台不受支持仍直接拒绝安装。

#### 4.3C 全局 Runtime 试切换与用户裁决（User confirmed）

**已确认方案二修正版：**任一稳定状态只有一个全局 committed Runtime，不提供 per-Plugin/per-MiniApp Runtime，也不允许长期 mixed-runtime。候选先完成 Node/Foundation 硬 probe；用户开始试切换后，先停止旧 Runtime 的全部 JavaScript Host，再使用候选 Runtime 验证能够诚实覆盖的 Plugin、MiniApp Service 和 Build Foundation，旧/新 Runtime 不同时承载 Host。

Runtime 状态只保留：

```text
selected_runtime
pending_candidate?
validation_result?
last_error?
```

候选若连 executable、identity、version、architecture 或 Host Foundation Hello 都无法通过，则不具备最低执行能力，不能进入局部影响裁决，旧 selected 保持不变。基础 probe 通过后，试切换流程固定为：

```text
暂停新JavaScript调用并有界drain
→ 停止全部Extension/Service/Build Host
→ 确认旧Runtime进程树为0
→ 使用候选Runtime做入口加载、注册、Service冷启动和Build Foundation验证
→ 汇总已成功、已失败、未覆盖对象
```

全部通过时直接提交 `selected_runtime = candidate`。出现局部失败时，先停止候选试运行 Host，再由用户二选一：

- **继续使用新 Runtime：**提交候选；可归因的失败 Plugin/MiniApp 保留产品身份但进入 `runtime_error`，不使用旧 Runtime；其他对象按新 Runtime 运行；
- **停止替换并恢复旧 Runtime：**selected 保持旧值，清理候选进程并用旧 Runtime 恢复原运行集合。

平台不自动替用户提交或回退。若 Extension Host 故障无法可靠归因到某个 Plugin，影响清单必须显示“全部 Plugin 能力受影响”，不能伪造逐 Plugin 精度。启动验证不调用业务 Tool，也不能证明未来所有惰性路径兼容；用户继续后若新路径后来失败，只把对应产品标记为 error 并提示，Runtime 不自动全局回退。

试切换和恢复只改变 Runtime 选择与进程，不撤销试启动期间已经产生的 KV、文件、SQLite、网络或外部命令副作用。用户提交候选前，持久化 selected 始终是旧 Runtime；若 NomiFun 中途崩溃，清理候选进程树，下次仍使用旧 Runtime并显示“上次试切换中断”。不建设长期 mixed-runtime、participant receipt、cohort generation 或自动 rollback 状态机。

#### 4.3D-1 Extension Host 惰性启动（User confirmed）

**已确认方案一：**Plugin Enable 只完成 Package、Config、Credential、Runtime 和 Contribution 的静态校验，状态变为 `enabled/available-on-demand`，但不启动 Node。第一次来自任一正式消费者的 capability demand、Candidate Test、显式 Retry 或 Runtime 全局试切换才启动 shared Extension Host；第一次启动失败时当前调用 typed failure，由用户 Retry/Restore。Host 启动后首版保持到 App 退出、generation 变化、全部 Plugin 停用或故障，不额外建设 idle shutdown，也不在每次 NomiFun 启动时自动执行第三方 Plugin 代码。

- trusted Plugin 主线程可以使用 Node 的公开 API，包括 network、timer、Worker、child process 和外部命令；Host 不做逐 API 拦截；
- 只有正式 `main.mjs` 主线程获得 Plugin Host Context；Worker/child 不获得第二份 Context，也不能自行注册 contribution；

#### 4.3D-2 Shared Host 内的 Mount 加载范围（User confirmed）

**已确认方案一：**shared Extension Host 由第一次真实 demand 创建后，只 import/activate 当前被请求的 Mount；后续其他 Plugin 第一次被请求时，动态加入同一个 Host，不创建新进程。`resident_mounts` 只作为 Host 内存事实，不持久化。未 resident Plugin 的 Replace/Disable/Restore 不重启 Host；resident Plugin 的变更为清理 raw 资源而结束整个 Host generation。普通 activation error 只影响当前 Mount；Runtime 全局试切换是显式例外，会验证全部 enabled Mount。

#### 4.3E Shared Extension Host 故障与整代回收（User confirmed）

**已确认方案一：**Plugin invocation 返回的 throw/rejection 只失败当前调用；`uncaughtException`、进程退出、IPC 断开或 watchdog 超时视为进程级 Host failure，立即失败当前 generation 内全部 in-flight/queued 调用、拒绝迟到结果并终止整棵 Node process tree。清空内存 resident Mount 后保持 stopped，不后台重启或重载旧 resident 集；下一次任一正式消费者的新 capability demand 或用户显式 Retry 才创建新 generation，并只加载本次请求的 Mount。

能够可靠归因时标记具体 Plugin；无法归因时显示全部 resident Plugin 受影响。故障调用永不自动重放，平台不自动 Restore/Disable，也不建设 restart backoff、breaker 或 resident assignment replay。MiniApp 使用 dedicated Service Host，其失败和 restart/backoff 策略由 §4.5 已确认合同定义。

### 4.4 MiniApp Build、Release 与 Publish

#### 4.4A MiniApp Source 与固定 Build 管线（User confirmed）

**已确认方案一并经 §4.7 扩展：**MiniApp M1 由 NomiFun 管理 owner-scoped Source，并使用固定 Build Profile 把 JS/TS、UI 资源和 pure-JS npm 依赖构建为 immutable Release output。依赖解析结果以 exact lock/digest 固化；Build 不执行用户自定义 build/install/postinstall script，不支持 native addon 或运行期 `node_modules`。Build 全程在 staging，失败只保留 Source 和日志，不改变 Ready/Active。外部工具链产出的合同兼容 Plugin Artifact 继续允许导入；NomiFun Chat Dev 使用同一 Foundation 的 `plugin-package-v1` Profile，两个入口最终都只产生和运行 prebuilt `main.mjs` Package。

M1 MiniApp Build 使用唯一固定管线：

```text
managed Source
  ├─ manifest
  ├─ ui/index.html + local JS/TS/CSS/assets
  ├─ service/main.js|ts       # optional
  └─ package.json             # optional pure-JS dependencies
        ↓ NomiFun-owned Build Host + fixed packer
immutable Release output
```

- Build 可以使用选定 Node，但不能执行 MiniApp 自定义的 `scripts.build`、postinstall 或任意安装脚本；
- 首版只正式支持 Node builtins 和 pure-JS npm dependencies；Build 解析 exact versions，并把 dependency lock/digest 固化为 Build input；
- UI 输出为自包含静态资源，Service 输出为自包含 `service/main.mjs`；运行期没有 `node_modules` 安装；
- Build 全程在 staging，成功才原子替换 Ready，失败保留旧 Ready 和 Active；
- 若 Release 声明 Capability contribution，Manifest 必须同时声明稳定 capability identity、supported consumer surface、typed resource/runtime contract 和 provenance；这些字段描述平台供给，不把 MiniApp 变成 Agent 专属 Package；
- 使用哪个 packer、lock 文件物理格式和缓存目录属于实现填充，但不得改变上述输入、输出和禁止脚本合同。

#### 4.4B Release Artifact 边界（User confirmed）

**已确认方案一：**MiniApp M1 每次成功 Build 只生成一个自包含、不可变的 Release Bundle，规范化 `manifest + ui/** + optional service/main.mjs` 后计算一个 canonical Release digest。Ready、Active、Previous、Publish、Rollback、Export 和 consumer provenance 都以完整 Release 为单位。UI/Service 可以有内部 content hash 和 `service_run_key` 用于缓存或避免无谓重启，但不成为独立产品 Artifact；不建设 Component refcount、同步 GC、独立发布或 component-level rollback。

```text
release/
  manifest.json
  ui/**
  service/main.mjs   # optional
```

#### 4.4C Publish 触发权（User confirmed）

**已确认方案二：**M1 默认人工 Publish；用户可以按具体 MiniApp 显式开启 `auto_publish_ui_changes`，Agent 或其他 authoring client 不能开启或扩大授权。只有平台能够完整证明 Service/service run key、Migration、Capability/Catalog、Bridge、Config、Credential、Resource、lifecycle、Runtime 和 dependency lock 全部不变，变化严格只属于 UI Source/Output 时，Build 成功才自动原子切换 Active；任何未知或不满足条件的变化只生成 Ready。首次发布及所有 Service/数据/合同变化始终人工；不建设观察窗口或自动 Rollback，失败由用户显式 Rollback。

#### 4.4D Publish Cutover 与 Service 停机（User confirmed）

**已确认方案一：**UI-only 或 `service_run_key` 未变化且没有 pending Migration 时复用现有 Service，只原子切换 Active/Catalog并 reload Surface。只要 Service 输入变化或存在 pending Migration，就 fence/drain并停止旧 Host，执行 additive Migration，再以唯一 unpublished target Host 启动目标 Service并通过基础 ready，之后才在一个事务中切换 Active/Previous/Catalog并绑定新 epoch。提交前任何失败都保持旧 Active并重启旧 Service；提交后故障由用户 Retry/Rollback。接受清晰可见的短暂停机，不同时运行旧、新 Service，也不建设双 Host candidate routing、容量预留、multi-generation drain 或通用 Transition 状态机。

```text
validate Ready
→ stop/fence affected old Service when needed
→ run pending DB Migration in one transaction when present
→ start the single unpublished target Service and wait for basic ready
→ one DB transaction swaps Active/Previous/Ready and materialized Catalog
→ bind the ready Service to the new active epoch
→ reload Surface and create a new Host-owned Bridge port
```

- 发布前失败不改变 Active；
- 新 Service ready 或指针/Catalog 事务在提交前失败时，终止 target Host、保持旧 Active/Catalog并重启旧 Service；如果此前 additive Migration 已提交，schema 保留，业务兼容性由 MiniApp 作者合同负责；
- Active 提交后的 Service failure 使 MiniApp 显示 error，由用户 Retry 或 Rollback；
- Rollback 也必须复用同一单 Host pre-ready cutover：不执行反向 Migration；先验证 Previous bytes、当前全局 Runtime 和当前 Config/Resource 下的 `ResolvedServiceSpec`，必要时停止当前 Service并用当前数据库启动 Previous target，只有 ready 后才原子交换 Active/Previous/Catalog。Rollback target 启动失败时保持当前 Active/Catalog并恢复当前 Service；
- Rollback 是新的显式代码/路由切换，不自动回滚数据，Previous 读取当前 additive schema 的兼容性仍由 MiniApp 作者合同负责；
- Release 切换关闭旧 Bridge port、取消旧 in-flight 调用并 reload Surface；
- 只保留一个 active release epoch 拒绝旧回调，不建设通用 multi-generation drain；
- 不建设通用 PromotionPolicy、风险评分、观察计时器、自动 rollback proof 或多阶段 rollout；仅保留已确认的 per-MiniApp 纯 UI boolean 授权和 fail-closed exact predicate。

AgentPreset Revision 作为一个消费者，按 05 §15 绑定稳定 `miniapp_id + contribution_id + contract_digest`，调用时解析当前兼容 Active Release并在 Snapshot 记录实际 release digest。其他消费者使用同义的 exact OperationLock，不需要创建 AgentPreset。只改 UI 时，只要 Service run key 未变化，可以复用现有 Service 进程。

#### 4.4E Ready、Active 与 Previous 保留规则（User confirmed）

**已确认方案一：**Runtime 只看到 immutable Release；Source 和 Build 是开发资产。每个 MiniApp 最多长期保留一个 `ready_release`、一个 `active_release` 和一个 `previous_release`。Build 成功原子替换旧 Ready，失败保留旧 Ready；成功 Publish 让旧 Active 成为 Previous并释放更老 Previous；成功 Rollback 按 §4.4D 的 pre-ready cutover 交换 Active/Previous，但不回滚数据，失败则保持当前 Active。Config/Credential/Runtime 等同 Release 变更不轮换。释放 owner ref 后物理 bytes 由启动或维护时 best-effort 清理；Build/Publish/Release digest 的轻量历史元数据可以保留，但 M1 不保存完整可执行 Release 历史、不支持任意历史 Rollback或 Pin。

### 4.5 MiniApp Service、Bridge 与数据

**保留确认：**一个 MiniApp 最多一个 `main` Service；UI-only MiniApp 不启动 Node；Service 支持 `on_demand` 与 `continuous`；UI 通过 Host-owned MessageChannel Bridge 调用，不开放 localhost。

#### 4.5A Service 复用与运行身份（User confirmed）

**已确认方案一：**§4.4D 已确认相同 Service 运行输入复用现有 Host，不同输入采用单 Host短暂停机重启。Service 运行身份只由 Host 实际消费的完整 canonical `ResolvedServiceSpec` 及其 `service_run_key` 表达；相同 spec 且 Host 健康时复用，不同 spec 或 Host 不健康时重启。run key 覆盖 Service bytes、Host/SDK protocol、全局 Runtime、Config、Resource、lifecycle、Bridge、平台 Capability exports、Storage 和平台架构等不可变启动输入，但不包含通过 Host 实时 resolve 的 Credential value/revision。运行时只保留当前 process、host generation 和 health，不建设持久化 ServiceMount、Deployment、ReleaseServiceBinding 或 Deployment history。

```text
service_run_key = digest(canonical ResolvedServiceSpec)
```

相同 key 且 Host 健康时可以复用进程，不同 key 或不健康 Host 就重启。Host 同时保存 canonical spec bytes 供比较和诊断，不只依赖 hash 字符串。

#### 4.5B Service 生命周期参数（User confirmed）

**已确认方案一：**Manifest 只选择 `on_demand | continuous`。on-demand 由首个 UI 调用或任一正式 Capability consumer demand 唤醒，在无 call/stream/subscription 后按平台固定 idle window 回收，raw timer 不阻止回收；continuous 在 enabled 时保持运行，进程故障使用平台统一的有限 backoff，达到固定阈值后进入 error 等待用户 Retry。startup timeout、idle timeout、backoff 和失败阈值在实现阶段冻结为平台常量，不向每个 MiniApp Manifest、UI 或 CLI 暴露，也不支持自定义 health check、keepalive、priority 或 restart policy。

Service 生命周期：

- `on_demand`：UI 或任一正式 Capability 消费者首次调用时启动，空闲后由内部策略回收；
- `continuous`：MiniApp enabled 后保持运行，crash 时按简单 backoff 重启；
- 正常运行的 trusted Service 可使用 Node 的公开 API、网络、timer、Worker 和 child process；Preview/Test 也不是安全 sandbox；
- 产品健康状态只有 `running | stopped | error`；
- 用户只需要 Stop/Start 或 Retry，不暴露 breaker、lease 和 generation 调参；

#### 4.5C Service Host 全局容量（User confirmed）

**已确认方案一：**所有 starting/running Service Host 共用一个全局 `max_active_service_hosts` semaphore；满额时返回 typed `service_capacity_exhausted` 并列出占用 MiniApp，由用户 Stop、调整 lifecycle、提高全局上限或 Retry。UI-only、stopped Service 和 Build Host 不计入；不自动驱逐、抢占、排队或设置 per-App priority/reservation。用户降低上限时不强杀现有 Host，只阻止新启动；默认值在实现阶段根据首发 required 平台测试冻结，并在 optional cell 实际交付时补充验证；允许用户在高级全局设置中调整为正整数。

- 所有 starting/running Service Host 共用该 semaphore；进程树归零后释放；
- 不做 8+1 reservation、transition slot、优先级、抢占或 durable lease。

#### 4.5D UI ↔ Service Bridge（Retained confirmed）

**保留确认：**UI 与 Service 只通过 Host-owned MessageChannel 通信，不开放 localhost；Channel scope、owner、active Release 和 Surface Session 全由 Host 绑定。UI-only KV 在 Rust Host 内终止，不启动 Node；Release 切换关闭旧 port、取消旧 in-flight并 reload Surface。

UI 与 Service 只通过 Host-owned MessageChannel 通信：

- wire 只包含 request ID、method 和 payload；
- `miniapp_id`、active release、owner 和 scope 全部由 Host 绑定在 port 上，不相信 UI 自报；
- stream 是 request 的辅助形式，不另建协议族；
- 不开放 localhost，不向 UI 泄露端口、token 或内部路径；
- UI-only MiniApp 的 KV 调用直接由 Host 处理，不启动 Node。

#### 4.5E MiniApp 受管存储边界（User confirmed）

**已确认方案一：**UI-only MiniApp 只有 Host KV 且不启动 Node；有 Service 的 MiniApp 获得 Host KV、一个稳定 owner-scoped `filesDir` 和一个 Host-managed Private SQLite。Files 由 Service 直接使用 raw `node:fs`，NomiFun 只负责归属、Export/Delete，不建设文件 pseudo-VFS；Private SQLite 路径不作为 SDK 合同，只通过 Host Database API 使用，并参与 Publish Migration、disabled Whole-App Backup Export 和 Permanent Delete。核心/其他 App DB 永不开放，受管目录外数据不进入保证；Plugin 继续维持 raw `dataDir/node:sqlite`、无 Host Database 的独立模式。

M1 数据能力：

| 存储 | UI-only | Service | 首版合同 |
|---|---:|---:|---|
| KV | 是 | 是 | owner-scoped、小对象状态 |
| Files | 否 | 是 | Service 的 owner-scoped 受管目录 |
| Private SQLite | 否 | 是 | Host 拥有路径，只经 DB API |

#### 4.5F Private SQLite API 与 Migration（User confirmed）

**已确认方案一：**Host-managed Private SQLite 运行期只提供参数化、单语句的 `query`（只读）、`execute`（DML）和有限 `batch`（Host-owned 单事务 DML）；不开放 DB path、ATTACH、extension、运行期 DDL、任意 PRAGMA 或用户持有 transaction。Release Migration 使用有序 immutable ID+digest ledger，在旧 Service 停止后以一个 transaction 执行，action allowlist 只含 CREATE TABLE/INDEX 和 ADD COLUMN，不含 DML/destructive migration；backfill 由新 Service 通过普通 DML API 幂等完成。不建设 schema diff、snapshot、双写或数据库时间回滚；高级任意 SQL 场景改用 `filesDir` 中自管 raw SQLite。

Private SQLite API 只提供：

- `query`：参数化、单条只读语句；
- `execute`：参数化、单条 DML；
- `batch`：有限条数、同一 Host transaction 的参数化 DML。

首版拒绝 ATTACH、extension loading、用户控制 transaction、访问核心/其他 App 数据库，以及在运行期直接 DDL。错误收敛为 `database_error`（携带 SQLite code、extended code 和清理后的 message）、`unavailable`、`timeout`、`canceled`、`result_too_large`、`quota_exceeded`。

Migration 是 Release 的有序、不可变 SQL 列表和一张 ledger：

- Publish 时先停止/fence 旧 Service；
- Host 在一个 SQLite transaction 中执行全部 pending Migration；
- Migration 的 action allowlist 只包含 CREATE TABLE/INDEX 和 ADD COLUMN；数据 backfill 不进入 Migration，若确有需要由新 Service 启动后通过正常 DML API 幂等执行；
- 向后兼容当前 Active/Previous 是 MiniApp 作者合同。NomiFun 只校验 action allowlist、owner、单事务和 SQL 能否执行，不证明业务语义兼容；
- 禁止删除/重命名既有表列。若作者违反兼容合同，旧 Release 仍可能业务失败，NomiFun 只保证旧 Active pointer 不被静默改写，不伪称已经自动恢复；
- Migration 失败时事务回滚，Active Release 不变；
- Migration、Release pointer 和 Catalog 位于不同事务边界；Migration 已提交后，即使 pointer 事务或新 Service 随后失败，additive schema 也会保留，代码 rollback 不能把它当作数据 rollback；
- 不做 dry-run 数据库副本、schema diff、迁移耗时估计、自动 pre-migration snapshot、dual write 或数据时间回滚。

#### 4.5G Preview/Test 数据边界（User confirmed）

**已确认方案一：**UI Preview 不启动 Service，只使用一次性 Preview KV。Service Test 由用户显式触发，先短暂停止生产 Service，再复制 Host KV 和 Private SQLite 到一次性 test namespace、创建空临时 `filesDir`、在测试 DB 上执行 Ready Migration，并以唯一 transient Test Host 运行；结束后回收进程并删除测试状态，再恢复 Active Service。默认不注入生产 Credential或正式 Effect，用户只能为单次 Test 显式确认使用当前 Credential。产品明确说明 raw fs/network/child process 副作用无法隔离；M1 不提供直接修改生产 KV/Files/DB 的 Live Test，也不建设 Files overlay、DB snapshot restore、网络 sandbox 或持久 Test Deployment。

**实现注记（2026-09-07，不改变阶段状态）：**内存 foundation 已用同一组 typed port
验证 §4.5A～§4.5F 的关键 fence：Bridge 保存并校验调用方交入的 exact committed
pointer，不合成 Catalog/pointer 事实；Release 或 Host generation 切换后的迟到结果优先
返回 stale；starting/running Host 共用动态正整数容量，降低上限不驱逐现有进程；
continuous 使用固定 `1s → 5s → error` 的有限重启策略，用户 Retry 清零；KV revision
overflow fail closed；Private DB 的 query/execute/batch 只允许取消在提交前胜出。该实现
仍是无生产路由的 deterministic foundation，后续必须由 migration 071+、SQLite
authorizer/transaction adapter、真实 dedicated Node process 和 Surface MessageChannel
adapter替换内存端口，不能直接声明 `M1-1-01` 或 `M1-1-02` 完成。

### 4.6 Operation、产品入口与交付 Gate

#### 4.6A Durable Operation 范围（User confirmed）

**已确认方案一：**只有 Build（MiniApp 或 Plugin Project）、Import、Export 和 MiniApp Permanent Delete 持久化为 Operation；状态只有 `running|succeeded|failed|canceled`。Build/Import/Export 可在提交前取消，失败或 App crash 后清理 staging并由用户重新提交；MiniApp Permanent Delete 写入 `deleting` 后不可取消。Enable/Disable/Trash/Restore/Apply Binding/Publish/Rollback/Plugin Candidate Apply/Runtime switch/Service Test 等不创建持久 Operation，只使用各自 pointer、ledger、瞬时 progress 和 Reconciler；不建设 queued/canceling/paused/retry_of、通用 phase engine 或长期大日志。

Phase N1/M1 durable Operation 只有：

- Build（MiniApp 或 Plugin Project）；
- Import；
- Export（包括 Whole-App export）；
- MiniApp Permanent Delete。

状态只有 `running | succeeded | failed | canceled`。Build、Import 和 Export 可以取消；MiniApp Permanent Delete 在写入 `deleting` 前可以放弃确认，写入后不可取消，只能幂等运行到成功或失败后重试。日志只保留固定尾部；重试就是重新提交，不保存通用 phase、`retry_of`、receipt digest 或 30 天 retention scheduler。

Enable、Disable、Trash、Restore-from-trash、Apply Binding、Publish 和 Rollback 都是短 DB transaction + Reconcile，不创建通用 Operation。

**实现填充——最小互斥与写入者归零：**每个 MiniApp，以及每组 linked Plugin Project/Mount，都只有一个 owner mutation lock；任何会写 owner 的 project source、dependency lock、Ready Candidate、product/config/binding/storage/release/target 状态，或者启动、停止 owner writer 的动作都必须取得同一把锁，只读动作例外。Build、Candidate Test/Apply、Project delete/unlink、Configure、Apply Binding、Publish、Rollback、Enable/Disable/Trash、Service Start/Stop、Export 和 Delete 都属于该规则；它不是持久 Operation 状态机，App crash 后由既有 pointer、ledger、Operation、Candidate 和 deleting intent 恢复。Runtime 全局试切换先等待所有相关 owner lock 释放，再取得 JavaScript Host 全局排他权；若仍有 Plugin/MiniApp Build/Test Host，必须列出影响并由用户选择取消这些任务或停止切换，不能静默 kill，取消后先写入明确终态并清理进程树。Permanent Delete 在写 `deleting` 前必须确认同 owner 的可取消任务已经由用户取消或自然结束，并回收所有 Service/Build/Test/transient Host；写入后拒绝新任务。Whole-App Backup Export 不仅要求 disabled，还必须在持有 owner lock 时由 Reconciler 证明 Service、Build、Test、Migration 和其他 owner writer 全部为 0，否则返回 typed `owner_busy`，不能复制正在变化的数据。

#### 4.6B MiniApp Permanent Delete 恢复粒度（User confirmed）

**已确认方案一：**MiniApp Permanent Delete 只在 owner 外持久化一条 `deleting` intent 和 last error；写入后立即撤销 Catalog/Surface/执行并不可取消。删除器每次都从头幂等扫描并删除 Source、staging、Release refs、Config、Credential bindings、KV、Files、Private DB 和其他 owner records，不存在即成功；失败则保留 intent并由启动 Reconciler或用户 Retry。全部受管数据完成后才删除 Product row和 intent；不持久化逐 Store phase/百分比，不建设 deletion journal，CAS physical bytes 继续 best-effort 后台清理。

Plugin 不复用这条“删除整个产品”语义：`plugin delete-data` 只删除 stable Mount 的 Config/KV/Credential bindings/`dataDir`并保留 Project；`plugin project delete` 只删除 Source/lock/Ready Candidate并保留已安装 Plugin和运行数据；组合删除必须在同一确认页逐项列出、分别授权，不能由任一单独命令级联。

Permanent Delete 流程：

```text
mark deleting
→ revoke Catalog and execution
→ close Surface and Bridge ports
→ stop/reap Service
→ idempotently delete owner data and refs
→ delete product row
```

崩溃后从头重扫；不记录每个 Store 的删除 phase。`operation cancel` 对已经进入 deleting 的任务返回 `not_cancelable`。CAS physical bytes 可以稍后清理。Whole-App Backup Export 要求 MiniApp 先 Disable，归档包含 Source、Release、非秘密配置、KV、Files 和 DB，但不包含明文 Credential 或本机 credential binding；Import 一律创建新的 `miniapp_id`，Credential slot 由用户重新绑定。

#### 4.6C Desktop 与 Headless 产品入口（User confirmed）

**已确认方案一：**保留完整用户动作，但收敛为 Plugin 列表/详情/Runtime 设置，以及 MiniApp Library/Workshop/Surface 等产品入口。内部 mount、run key、generation、process、ledger、semaphore 只进入所属产品的折叠诊断，不形成独立管理中心。Headless 使用按 Plugin、Runtime、Credential、MiniApp、Operation 分组的本地 CLI，覆盖 breaking Replace、retained restore、Runtime 继续/停止切换、Source/Build/Test/Publish/Rollback、Service Start/Stop/Retry、Import/Export/Delete，统一 JSON 和 `0/1/2` exit code；不建设 Admin HTTP 或 Deployment/Artifact/Generation/Capacity 底层操作命令，CLI 与 Desktop 共用 application service。

Plugin Desktop 首版只有：

- Plugin/Project Library，可发现已安装 Plugin、linked Project 和尚未安装的 unlinked draft；
- Plugin 详情/配置/数据状态，以及 Plugin Project 的 Chat Dev、Source/Build/Test/Ready Candidate/Apply/Discard/auto-apply 授权；
- Runtime 设置；
- 创建/Import/Export、Build/Test、安装/替换/恢复上一版/启停/卸载/删除数据动作。

MiniApp Desktop 首版只有：

- MiniApp Library；
- Chat Dev、Source/Build/Test 和 Ready Release；
- Publish/Rollback/Enable/Disable/Trash；
- 打开的 MiniApp Surface；
- Service 状态与 Retry；
- 无用户数据的 Share Import/Export、disabled Whole-App Backup/Import-as-new，以及 Delete。

Agent 工作台不是第三个 Plugin/MiniApp 管理中心。它只消费已经正式物化、对 `agent` Surface 兼容的 Capability/Skill，显示 provenance、availability 和 impact；安装、配置、Credential、Apply、Publish、Rollback、`dataDir` 和 Runtime 仍回到 Plugin/MiniApp/Runtime 所属产品入口。

Headless CLI 收敛为以下产品命令族：

```text
plugin list|show|install|configure|enable|disable|replace|retry|restore|uninstall|delete-data
plugin project list|create|show|source-path|delete
plugin build
plugin test
plugin import
plugin share export
plugin candidate show|discard|apply
plugin auto-apply enable|disable
runtime status|download|switch|switch-continue|switch-abort
credential list|put|delete

miniapp list|show|create|configure|source-path
miniapp build|test|publish|discard-ready|rollback
miniapp enable|disable|trash|restore|delete
miniapp share import|export
miniapp backup export|import-as-new
miniapp service start|stop|retry
operation list|show|cancel
```

CLI 与 Desktop 调用同一 application service。退出码只保留 `0=成功`、`1=操作失败`、`2=输入/用法错误`，具体原因放在 JSON `error.code`。

#### 4.6D 平台优先级、Gate 与 Evidence 分层（User confirmed）

**已确认方案一并调整交付优先级：**首发 required cells 只有三个，按 `Windows x64 → macOS arm64 → Linux x64 Desktop` 依次完成；macOS x64 与 Linux x64 Headless 是最低优先 optional cells，可以不随首版交付，也不阻塞首版 RC/Stable。未交付 optional cell 必须明确标记 `not_delivered/not_in_release_scope`，下载页和支持矩阵不得宣称支持。完整 Contract/Integration/Fault/Product suite 集中在代表性平台，其他 required 平台运行自身职责和真实 signed RC smoke，不重复所有组合测试。

交付证据只保留一份 `cohort-lock.json` 和一种 `PlatformValidationManifest(stage=candidate|signed_rc, requirement=required|optional)`。Candidate 与 signed RC 是不同记录，但不建设两套 Evidence Schema。首版 Stable 只要求同一 cohort 下三个 required cells 完成；optional cell 以后进入实际交付范围时，必须在当时的新 cohort 上完成自己的 Candidate + signed RC Gate，不能借用旧 required evidence冒充已交付。

### 4.7 Plugin Authoring 与 Self-Evolution（User confirmed）

**已确认决策并按一站式体验收敛：**NomiFun Chat Dev Mode 是 Plugin 与 MiniApp 的默认完整开发入口；同时正式支持外部 IDE/AI/工具链产出符合 Artifact 合同的 prebuilt Plugin/MiniApp 包并导入。所有来源进入同一 validator、Candidate Test 和 Apply/Publish 主链，不建设第二套 Runtime/安装路径，也不承诺复现每一种外部构建工具。Plugin Project 复用 JavaScript Authoring & Build Foundation，生成一个 immutable Ready Package Candidate，并可在 transient Candidate Test Host 中验证。默认 `ask_before_apply`；用户可以按 Plugin 开启 `auto_compatible_when_idle`，Agent 和 Plugin 自身不能开启或扩大授权。Breaking、dependency lock、Runtime、Config、Credential、Resource、Host/SDK 或平台合同变化永远转人工；自动部署只在 shared Host quiescent 时执行，失败沿用 Retry/Restore，不建设热更新或自动回滚。

这里的 Agent 是 Project-scoped authoring client，不是 Plugin 的 owner。它只能在用户打开的 Project scope 内编辑 Source、发起 Build/Test 和提交 Apply/Publish 请求；Package mutation、部署授权和正式 Capability 发布仍由 Plugin/MiniApp application service 与用户确认控制。

#### 4.7.1 共享 Build Foundation，不合并产品身份

```text
JavaScript Authoring & Build Foundation
  ├─ plugin-package-v1
  │    └─ manifest.json + main.mjs + resources/**
  └─ miniapp-release-v1
       └─ manifest.json + ui/** + optional service/main.mjs
```

两种 Profile 共享 Source Store、JS/TS、pure-JS npm resolver/cache、exact dependency lock/digest、fixed packer、Build Host、Build Operation、staging、日志、取消、immutable output 和 canonical digest。Plugin 不获得 MiniApp Surface、Release/Publish、managed DB 或 continuous Service；MiniApp 也不获得 Plugin Mount/Contribution 安装语义。

#### 4.7.2 NomiFun Plugin Project 与单 Ready Candidate

NomiFun 内开发从 Plugin Project 开始。外部开发者也可以直接导入符合 `plugin-package-v1` 的 prebuilt Artifact：包含 Source/Project metadata 的 Share Bundle 会创建可继续开发的 Project；只有 runtime Artifact 时仍可完成 Test、Install/Replace 和使用，但没有可编辑 Source。用户要继续开发时需显式导入 Source 或创建 editable fork，平台不从 `main.mjs` 假装还原原始 TypeScript 工程。Project Library 必须能发现 linked 与 unlinked draft Project，不能只从已安装 Mount 进入。

Plugin Project 最小事实：

```text
project_id
linked_mount_id?
managed_source?             # runtime-only external Artifact 时 absent
source_digest?
dependency_lock_digest?
ready_candidate?
```

同一 Project 同时最多一个 active Build；每次 Build 捕获不可变 Source/lock snapshot 和单调 build generation。Build 成功只生成一个非运行 Ready Candidate；新的成功 Build 通过 generation CAS 原子替换旧 Candidate，Build 失败保留 Source、旧 Candidate、current 和 previous。所有 Candidate 携带 `candidate_id + candidate_digest + base_target_digest + contract_diff + build/import result + matching_test_receipt_ref?`；authored Candidate 还必须携带真实 `source_snapshot_digest + dependency_lock_digest + build_profile_version`，runtime-only Candidate 对这些字段明确为 absent。Project head 在 Build 后继续变化不会修改 Candidate，但 Candidate 必须显示“Source 已继续变化”；自动 Apply 只允许 authored Candidate snapshot 仍等于当前 Project head。Apply 前若 `base_target_digest != current_target.digest`，Candidate 标记 stale并重新计算 diff，不能覆盖并发产生的新 current。

Runtime-only 外部 Artifact 不伪造空 Source/lock sentinel 为有效 Project snapshot；它创建 `source/lock=absent` 的只读 Candidate，可在本机 Test 后由用户 Manual Install/Replace，但永远不满足 `auto_compatible_when_idle`。用户后来导入 Source 或创建 fork 时，才形成新的真实 Source snapshot 和 Build lineage。

首次 Apply 时，如果 Project 尚未 linked 且 Package ID 没有 active/retained owner，Host 在同一个 owner/mutation transaction 中完成 install preflight、创建 stable Mount/current、link Project 和清除 exact Candidate。若同 Package ID 已 active，必须展示当前 target和影响，由用户选择 link 后 Replace或修改 Package ID；若存在 retained Mount，必须让用户选择显式 Restore retained data并 link，或先删除运行数据/修改 Package ID。任何路径都不能静默关联、创建第二个 Mount或覆盖 retained data。

Source、运行代码和数据严格分开：

```text
Plugin Project Source     # 作者资产
Ready Package Candidate   # 非运行候选
current/previous target   # 正式运行代码
Config/KV/Credential/dataDir # 稳定Mount数据
```

运行中的 Plugin 没有 Source 写入、Build、`replaceSelf()`、current/previous pointer 或其他 Plugin 管理 API。Agent 只能通过 Project-scoped authoring application service 编辑、Build、读取错误、查看 diff 和发起 Apply 请求。

#### 4.7.3 两种 Apply Mode

```text
ask_before_apply              # 默认
auto_compatible_when_idle     # 用户按Plugin显式授权
```

`ask_before_apply` 下，Build 成功只显示 Ready、digest、contract/dependency diff 和受影响消费者（包括 Agent、自动化、Gateway/Remote 等）；用户点击 Apply 后复用 §4.2 Replace。首次安装和所有 Breaking Apply 永远人工。

`auto_compatible_when_idle` 只能由用户针对 exact linked Mount 开启、随时关闭；Agent、Plugin、导入包和远程来源都不能修改。授权页面必须明确：contract-compatible 不等于行为、费用、网络副作用或 `dataDir` 兼容，连续自动 Apply 仍只保留一代 previous。

#### 4.7.4 Auto-Apply Exact Eligibility

只有以下事实全部可由 Host 完整证明时才允许自动 Apply：

- exact linked `mount_id` 和 Package ID 不变；
- Candidate base 仍等于 current digest；
- authored Source/lock/build profile lineage 完整存在，不是 runtime-only Artifact；
- Candidate 的 Source/lock snapshot 仍等于当前 Project head；
- 本机 Host 生成的 Candidate Test 已通过，并绑定 Candidate digest、当前 committed Runtime、OS/arch、Host/SDK contract 和 Test contract；
- Contribution exact-set、类型和全部 `contract_digest` 不变；
- Config Schema、Credential slots、Resource/Effect contract 不变；
- Host/SDK contract、Runtime requirement、supported platform 不变且当前 Runtime满足；
- dependency lock/digest 不变；
- Package/Manifest/static validator 全部通过；
- 没有未知、缺失或无法比较字段。

任一条件不满足时 Candidate 保持 Ready并转人工 Apply；Breaking 继续显示受影响消费者并要求现有 `--allow-breaking` 确认。Auto-Apply 不自动下载/切换 Runtime，不自动修改 Config/Credential/Resource binding，也不属于 Marketplace/Registry updater。

真正提交前必须在 owner mutation lock、Runtime global gate 和（resident 时）shared Host quiescent fence 的同一临界区重新计算并验证本节完整 eligibility，而不是只重查部分字段：exact Candidate/Test、Source/lock/build profile、Project head、base/current、授权、全部 contract/dependency/config/resource/platform/Runtime/unknown predicate 都必须仍然成立。`current→previous`、`candidate→current` 和只清除该 Candidate 必须原子完成；任一比较失败都保留 Ready并重新派生状态。

#### 4.7.5 Quiescent Deploy

目标 Mount 未 resident 时，Reconciler 在取得 Project/Mount mutation lock 后完成上一节的原子 CAS，不启动 Node。

目标 Mount 已 resident 时：

```text
原子try-acquire shared Host quiescent lock/fence
→ 仅在全部in-flight/queued调用为0时成功
→ 原子轮换current/previous并清空Ready
→ fence旧Host generation
→ terminate/reap whole process tree
→ 保持stopped
→ 下一次真实demand按新current启动
```

不能先观察为 0 再晚一步取锁，以免新调用插入；quiescent 判断和 fence 必须原子完成。不能在有业务调用时自动中断。若在有界时间内无法取得 quiescent lock，Candidate 保持 Ready并显示“等待 Extension Host 空闲”；用户可以继续等待、停止相关工作，或显式 Apply Now并确认影响。Reconciler 只在 Build success、Candidate Test success、Host quiescent、App 启动且 Host 尚未创建、用户动作等事件点尝试，不建设 polling updater/scheduler。

自动 Apply 成功只产生非阻断通知和审计事件，记录 old/new Version、digest、Source/Build、时间和授权来源，并提供 Restore Previous。其他 resident Plugin 下次使用时可能冷启动，但没有正在执行的调用被自动中断。

#### 4.7.6 Candidate Test 与 Apply 后失败

Build 或外部 Artifact Import 通过静态 validator 后，用户或 Project-scoped authoring client 可以在 Chat Dev Mode 中显式 Test Ready Candidate；测试不改变 current、Catalog 或任何正式 consumer lock。

```text
immutable Ready Candidate
→ 创建一个transient dedicated Plugin Test Host
→ 绑定一次性KV和临时dataDir
→ 加载/activate Candidate
→ 由Chat Dev调用声明的Test/Tool场景
→ 记录与Candidate digest绑定的结果和日志
→ terminate/reap完整process tree
→ 删除临时受管状态
```

Test Host 不进入 shared Extension Host，不加载其他 Plugin，也不形成持久 Candidate Deployment。默认不注入生产 Credential 或正式 Effect；用户只能针对单次 Test 明确确认使用当前 Credential binding。由于 Candidate 仍是 trusted raw Node code，NomiFun 不能阻止其扫描 OS 文件、访问网络、运行 child process 或产生外部副作用，测试入口必须明确提示，不能宣传为安全 sandbox。

Candidate Test 的唯一权威是本机 Host-issued `CandidateTestReceipt`，以 exact `candidate_id + candidate_digest` 为键，并绑定可选 Source/lock snapshot、selected Runtime fingerprint、OS/arch、Host/SDK contract、Test contract 和测试输入模式；Ready Candidate 最多只引用一个 matching receipt，Project 不另存独立可变 Test result。任一事实变化立即使引用失效。导入包携带的 Test 只作 provenance，不能关闭本机 Gate。Test 所需 Config、Resource、fixture 或一次性 Credential 不完整时显示 `needs_test_input`，不得 auto-Apply。Test failure 只影响 Candidate，current/previous 保持不变。App crash 或取消时回收 Test process tree 和临时状态；Test 是瞬时验证，不加入 Durable Operation。

Chat Dev 默认主流程固定为 Build → Test → Apply，测试失败或缺输入时主操作保持“修复/补充测试输入”。Auto-Apply 必须有 matching passed Test。Manual Apply 仍允许用户通过折叠的高级动作明确越过“未通过/未运行 Test”警告，因为 Test 不是安全证明；Breaking impact 仍需单独确认。Candidate Test 只能证明所执行场景，不保证所有未来 Tool 路径、生产 Credential、生产 `dataDir` 或外部系统行为兼容。

Apply 本身不再次执行业务 Tool。下一次正式 demand 的 lazy activation 仍可能失败；此时新 target 保持 current/error，用户收到 Retry/Restore 提示，不自动回滚、重放调用或恢复数据。Restore 仍只回退代码，不回退 Config、Credential、KV、`dataDir`、raw SQLite 或外部副作用。

#### 4.7.7 Chat Dev、Export 与 Import 闭环

Plugin 与 MiniApp 共用一个 chat-first Project Workshop：Chat 是主入口，文件树、diff、Build/Test 日志、Ready 影响和 Apply/Publish 是辅助视图，不建设第二套完整 IDE、终端或包管理器 UI。

```text
创建/导入Project
→ Chat中描述需求
→ Project-scoped authoring client（通常由 Agent 驱动）编辑Source和依赖
→ Build
→ Test
→ 查看影响
→ Plugin Apply / MiniApp Publish
→ Export Share Bundle
```

首次没有兼容 Node 时，Project 创建仍可完成；第一次 Build/Test 前由 Runtime Manager 显示一次官方 LTS 下载确认，之后自动完成下载、校验、选择和 Build Host配置。用户不需要手工安装 Node/npm/TypeScript/packer/SDK，也不需要复制命令。

正式支持两类导入：

- **NomiFun Share Bundle：**包含 Project Source、dependency lock、Build Profile version、immutable Artifact、Manifest和最小 Test/provenance metadata，导入后可以验证、使用并继续开发；Share manifest 必须固定并校验 `source snapshot digest + lock digest + build profile version + artifact digest` 完整链，不一致时不得把 Source与Artifact关联为可继续开发的同一 Project，只能拒绝或分别按 runtime-only Artifact 与 detached Source处理；默认不包含用户 Credential、KV、`dataDir`、Files 或 Private DB；Test 默认只导出 pass/fail、digest、平台/Runtime/Test contract 等元数据，不导出测试输入、输出、API响应或日志，诊断内容必须由用户另行审阅导出；
- **Prebuilt Runtime Artifact：**由外部 IDE/AI/工具链按公开 Artifact Schema/SDK types/validator 合同生成；导入后可 Test、Install/Replace 或 Publish，但没有 Source 时只能使用，继续开发需显式导入 Source 或创建 fork。

Plugin Share Bundle 或 prebuilt Artifact 导入后统一进入 Ready Candidate和既有 diff/Apply 流程；首次安装必须人工。MiniApp Share Bundle 或 source-less prebuilt Release 一律创建新的 MiniApp identity和 Ready Release，不静默覆盖现有 App；包含 Source 时同时创建 editable Project，source-less 时明确标记 read-only artifact，仍可本机 Test/Publish，继续开发需导入 Source或 fork。独立 disabled Whole-App Backup Export 已按本合同实现为携带业务数据的 Backup/迁移包，与默认无用户数据的 Share Bundle 分开；两者都不包含明文 Credential，导入后重新绑定。

Share Bundle 中携带的 Test result 只作为来源 provenance 展示，不能直接满足本机 auto-Apply/Publish eligibility；导入后必须用本机当前 committed Runtime 和本机 Test contract 重新得到 matching passed result。外部或分享产物可以在用户明确确认下跳过 Test手动应用，但不能凭来源自述获得自动部署资格。

远程 Registry updater、完整历史、LKG、观察窗口、风险评分、双 Host、多版本运行和 hot reload 继续后置。

## 5. 最小状态与恢复规则

### 5.1 必须持久化的事实

| 领域 | 最小持久化事实 |
|---|---|
| Plugin | package identity、current/previous target、enabled、config、credential bindings、retained/delete-pending data、last error；Project Source、dependency lock、single Ready Candidate、base/source digests、matching CandidateTestReceipt ref 与 apply mode |
| Runtime | selected、pending candidate、validation result、last error、one-time warning acknowledgement；稳定态只有一个 committed Runtime |
| Platform Capability Catalog | 已发布且已物化的 Capability/Skill、owner/provenance、supported consumers、contract digest、availability；不包含 Candidate、未发布 Release 或 Agent 专属副本 |
| AgentPreset | identity、current revision、immutable Revision payload、ContributionLock、revision digest；不保存 Plugin/MiniApp 私有数据或 Runtime 选择 |
| MiniApp | product identity、enabled/trash/deleting、active/previous/ready release、config/bindings；project_id、managed_source?/source_digest?/dependency_lock_digest?、build_profile_version 与 Ready test/provenance ref。source-less prebuilt 对作者字段明确为 absent/read-only；作者事实不取代 Active Release |
| MiniApp DB | migration ledger |
| Long task | kind、owner、state、progress、last error、bounded log tail |
| Release evidence | cohort lock、per-platform validation manifest |

其他状态优先从这些事实派生，不新增平行的 desired/effective/LKG/health/recovery/receipt 图。

### 5.2 统一失败原则

- staging/validate/build 失败：已发布事实不变；
- 持久化指针（Plugin current target 或 MiniApp Active Release）切换失败：事务回滚；
- 指针已发布、finalize 未完成：published pointer 权威，Reconciler 补齐；
- shared Extension Host 丢失：失败当前 generation 的全部调用并回收整棵进程树；保持 stopped，直到下一次新 demand 或用户 Retry；
- 单个 MiniApp Service 启动失败：只影响该 App；
- 删除中断：根据 `deleting` 或 `delete_pending` 幂等重扫；
- 数据副作用已经提交：代码 rollback 不伪装成数据 rollback。

需要做 crash injection 的公共边界只有三条：

1. 任何持久副作用开始之前；
2. staging/Candidate Test/Migration 已完成，但 Plugin current 或 MiniApp Active pointer 尚未发布；
3. Plugin current 或 MiniApp Active pointer 已发布，但 Reconcile/finalize 尚未完成。

带 Migration 的 Publish 完整覆盖三条；Migration 一旦提交，后续失败也可能保留 additive schema，Release rollback 只回退代码与路由。Plugin manual/auto Candidate Apply 至少各覆盖一条代表性提交前/后 crash：提交前 current/previous 不变且 Candidate仍 Ready；提交后新 current 权威，Reconciler fence/reap旧 Host generation并清除 exact Candidate，不自动回滚。其余短动作使用 table-driven unit test 和一个代表性 E2E，不做“操作种类 × 阶段 × 平台 × 故障”的笛卡尔积。

## 6. 实施阶段

本节的依赖不是并行关系：

```text
05 §15 AP-0～AP-7
        │
        ├─ Agent 工作台 / AgentPreset 主链可用
        ├─ 平台 Capability Catalog 已有 Agent + 非 Agent 消费者
        └─ 旧 Preset 生产可达性为 0
        ↓
N1-0 机器合同冻结
        ↓
N1-1/N1-2 Plugin Runtime 与 Package 生命周期
        ↓
N1-3 平台 Catalog + 消费者集成
        ↓
N1-4 Plugin Authoring / Self-Evolution
        ↓
M1 MiniApp
```

### N1-0：前置门禁复核与简化合同冻结

交付：

- 核验 05 §15 的 AP-0～AP-7 已全部完成；若任一项未完成，只能继续设计审阅，不得进入本阶段代码；
- 将 §4 七组已确认合同转录为机器可验的 Schema、IPC、Manifest 与 Gate；
- 冻结 Package Manifest、Plugin SDK、Host IPC、Runtime probe、Contribution 和 CLI JSON schema；
- 冻结 Project/Source snapshot、Build Profile、Ready Candidate、Candidate Test result、Apply mode/eligibility、Share Bundle/Prebuilt Import 与 quiescent CAS 合同；
- 冻结只读官方 Agent creation seed；不为 Package-owned AgentPreset template 建立 N1 admission；
- 将官方 `agent_preset_templates` 与 `/api/agent-preset-templates/*` 限定为 seed-only 读取，不把模板当作可执行 Capability、Package owner 或 Runtime profile；
- 不在本阶段重新定义 AgentPreset、旧 Preset 兼容层或 Agent 专属 Catalog；Agent integration 只引用 05 §15 已冻结的 application service 和 Consumer contract；
- 明确 Stable v2 与 Phase N1 的契约边界；
- 建立 `cohort-lock.json` 与 conformance fixture 目录。

退出条件：AP-0～AP-7 admission gate 全部通过；生产代码、Schema、route 和 feature flag 不再引用已废弃的 Phase N 实现（历史设计文档可保留，但必须明确以 05/06 覆盖）；N1-0 的代码不依赖旧 `/api/presets`。

### N1-1：Node Foundation 与最小 Tool Sentinel

交付：

- 薄 Runtime Manager；
- official Node 下载和外部 Node probe；
- lazy shared Extension Host、private IPC、watchdog、whole-host cleanup；
- 一个 prebuilt `main.mjs` Tool 插件；
- JS/TS SDK 与开发者构建模板。

退出条件：没有 capability demand 时 Node process 为 0；首次调用可启动和调用，crash 后进程树归零，重试或新 demand 可按当前事实恢复。

### N1-2：Package 生命周期与数据

交付：

- staging/containment/digest/atomic inventory；
- install/replace/restore/uninstall/delete-data；
- Config、KV/CAS、Credential slot、稳定 `dataDir`；
- Desktop application service 和 Headless CLI 共用后端。

退出条件：失败替换不破坏 current；卸载默认保留数据；显式删除可在崩溃后幂等完成。

### N1-3：平台 Catalog、消费者集成与参考插件

交付：

- 五类 Contribution materialization；
- 平台 Capability Catalog、owner/provenance/availability 和 supported consumer surface；
- 至少一个非 Agent consumer adapter，以及对 05 §15 Agent 工作台、Revision、Snapshot、Runtime invoke 的接入；
- 一个同时支持 Agent/非 Agent 的共享 Capability，以及一个明确 non-Agent-only 的参考 contribution；
- CSV/JSON Tool 参考插件；
- npm Connector、Worker/child、timer、exception/hang/crash 独立 fixture。

退出条件：Agent 和至少一个非 Agent 消费者都能选择并固定共享 contribution contract，Agent picker 正确过滤 non-Agent-only contribution；升级后继续解析当前兼容 Mount；核心 ID 与 UI 无 override 路径；没有 `contributes.presets`、Package-owned AgentPreset template 或 Agent-only Catalog。

### N1-4：JavaScript Authoring Foundation 与 Plugin Self-Evolution

交付：

- 抽取共享 Source Store、pure-JS npm resolver/cache、exact lock/digest、fixed Build Host/packer、Build Operation、staging 和日志；
- 冻结 `plugin-package-v1` 与 `miniapp-release-v1` 两个 Build Profile；
- Plugin Project Library、Project-scoped Agent authoring（仅作为受限 authoring consumer）、外部 Artifact/Source Import、single Ready Candidate、base-target stale check 和 contract/dependency impact diff；
- Chat Dev Mode、transient Candidate Test Host、临时 KV/dataDir、单次 Credential 确认、matching Test result 和完整进程清理；
- `ask_before_apply | auto_compatible_when_idle`，Breaking/unknown 统一转人工；
- non-resident 原子 Apply、resident shared Host quiescent Apply、等待空闲、通知、Retry/Restore；
- Plugin Workshop、NomiFun Share Bundle、外部 prebuilt Artifact Import，以及 Headless project/build/test/candidate/auto-apply/import/export 命令。

退出条件：用户只安装 NomiFun 即可在 Chat Dev Mode 创建/修改 JS 与 TS Plugin Project、解析依赖、修复 Build、用临时受管状态 Test并生成一个 immutable Candidate；手动 Apply 与授权 compatible idle Apply 都复用同一 validator/Replace/current/previous，busy 不强杀调用，activation failure 不自动回滚。NomiFun Share Bundle 与符合 Artifact合同的外部 prebuilt 包都能导入同一 Test/Apply 主链；remote updater、第二 Runtime/安装路径、hot reload residual 为 0；Agent authoring 不获得 Package owner 或部署授权。

### N1-5：Windows Plugin Candidate

交付：

- Windows whole-candidate 完整合同、集成、故障、产品和 accessibility；
- Windows NSIS 安装版完成 authored Plugin、Candidate Test、manual/idle Apply、调用、
  Restore、Host-loss、卸载和 digest smoke；
- 记录 N1 Windows 结果，但此时不冻结跨平台最终 cohort；
- macOS x64、Linux x64 Headless 仅登记为最低优先 optional backlog。

退出条件：Windows N1 功能与候选 Gate 通过，M1 可以复用同一机器合同与 Authoring
Foundation 开始实施；macOS/Linux 尚未执行，不冒充 Stable。

### M1-0：Clean-start、UI-only MiniApp 与 Release

交付：

- 全新 MiniApp 数据根和产品身份；
- `static_bundle_v1` authoring，单 `index.html` 输入打包为自包含 UI；
- 复用 N1-4 已交付的 JavaScript Authoring & Build Foundation，通过 `miniapp-release-v1` 完成 JS/TS/pure-JS npm bundling、dependency lock/digest 和禁止自定义脚本合同；
- MiniApp Chat Dev Project 与 Project-scoped Agent edit/build 能力；
- Source/Build/Ready/Active/Previous；
- Library、Surface、默认 manual Publish、可选纯 UI auto-publish、Rollback、Host KV；
- 旧 MiniApp 数据不读取、不迁移，也不在升级过程中自动删除。

退出条件：Build 可以短暂使用 Build Host；Build 完成后，UI-only MiniApp 的打开、持久化状态和回滚全程不启动 Node Runtime process。

### M1-1：单 Service、Bridge 与受管数据

交付：

- optional `service/main.mjs`；
- dedicated Service Host、run key、on-demand/continuous；
- MessageChannel Bridge；
- KV/Files/Private SQLite、Migration ledger；
- simple capacity semaphore 和 Service Retry。

退出条件：UI 和至少一个正式 Capability consumer 可以调用当前 Active Service；Release 切换不接受旧 port 回调；一个 App 故障不影响其他 App。

### M1-2：产品生命周期、导入导出与 Gate

交付：

- Enable/Disable/Trash/Restore-from-trash/Permanent Delete；
- 默认无用户数据的 Share Bundle/外部 prebuilt Artifact Import，以及与其分离的 disabled Whole-App Backup export/import-as-new；
- durable Build/Import/Export/Delete Operation；
- Catalog 原子发布；
- MiniApp Windows Candidate 证据；optional cells 后续独立关闭。

退出条件：删除可重扫，Export 不含明文 Credential，Import 生成新 ID；Release pointer 与 Catalog 不出现半状态，已经提交的 backward-compatible additive schema 不伪装成可回滚。

### M1-3：最终 Windows cohort 与 required 原生平台交接

交付：

- 在 N1/M1 Windows 全部通过后构建一次最终 Windows NSIS，完成安装、启动、代表性
  Plugin/MiniApp、Host-loss、卸载、release lock 和 digest smoke；
- 冻结同一 source cohort 与平台输入；
- 将该 cohort 统一交给 macOS arm64 与 Linux Desktop x64，分别完成自身原生职责；
- required 三平台通过后原样提升 RC/Stable，不重建 bytes；
- macOS x64、Linux x64 Headless 保持 optional/not-delivered。

退出条件：Windows x64、macOS arm64、Linux Desktop x64 对同一最终 cohort 形成真实
制品证据；支持矩阵不宣称未交付 optional cells。

## 7. 验证矩阵

| 层级 | 必须验证 | 不重复验证 |
|---|---|---|
| Contract/Unit | Schema、canonical serialization、owner namespace、atomic store、DB SQL boundary | UI 旅程 |
| Agent admission | 05 §15 AP-0～AP-7、AgentPreset Compiler、ContributionLock、Snapshot exactness、旧 `/api/presets` reachability | Plugin Host 内部实现 |
| Consumer integration | 同一 Capability 被 Agent 与至少一个非 Agent 消费者解析；non-Agent-only contribution 不进入 Agent picker；impact 覆盖 Agent Revision、Binding、Automation、Gateway/Remote operation lock | 各消费者重复实现同一 Package 解析 |
| Windows candidate | 完整 Plugin/MiniApp 闭环、核心 fault、UI/accessibility | Node 自身每个 builtin 的语义 |
| macOS arm64 candidate | Runtime、Package、Host、Plugin/MiniApp 核心闭环 | 全部 SQL 负向矩阵 |
| Linux x64 Desktop candidate | 原生 Desktop/Surface/路径/权限/IPC/cleanup，并完成本地 CLI 全生命周期和 JSON errors | Windows 已覆盖的完整 fault |
| required signed RC 三格 | Windows x64、macOS arm64、Linux x64 Desktop 的签名制品安装/升级/启动、代表性 Plugin/MiniApp、host-loss、卸载、digest | Candidate 全套重跑 |
| optional macOS x64 | 实际决定交付时运行架构、包装、启动、IPC、路径、cleanup、代表性产品闭环和 signed RC | 不阻塞首发 required RC |
| optional Linux x64 Headless | 实际决定交付时运行独立 Headless 制品、完整 CLI 生命周期、服务重启、cleanup、JSON errors 和 signed RC | 不阻塞首发 required RC |

Node 代表性 fixture 只保留：

- 一个纯 Tool；
- 一个依赖常见 npm package 的 Connector；
- child process；
- Worker；
- HTTP/WebSocket；
- exception、无限循环、Host crash 和未清理资源。

NomiFun 验证 Host 合同、进程回收和产品恢复，不重新证明 Node 的 `wasi/vm/inspector/cluster` 或每种网络协议实现正确。

Plugin Self-Evolution 代表集必须覆盖：

- External Prebuilt 与 NomiFun-authored JS/TS Project 产出相同 `plugin-package-v1` Artifact合同；
- pure-JS dependency lock、Build success/failure/cancel、single Ready replacement 和 staging cleanup；
- Candidate base stale 后不覆盖新 current；
- manual Apply、Breaking impact 和 Agent Revision/非 Agent consumer lock contract continuity；
-非 resident compatible auto-Apply；
- resident Host busy 时保留 Ready、不取消调用，quiescent 后原子 Apply并整代回收；
- dependency/Runtime/Config/Credential/Resource/contract/unknown diff 强制转人工；
- Agent/Plugin 不能开启授权，首次安装不能自动；Agent 也不能授予自己 Package、Plugin 或 MiniApp owner 权限；
- auto-Apply 后 lazy activation failure 保持 current/error，由用户 Retry/Restore，不自动回滚；
- Candidate Test 只使用单 transient dedicated Host并在每次结束后 process tree/临时受管状态为 0；持久 Candidate Deployment、remote updater、hot reload 和多版本运行 residual 为 0。

平台分配固定为：Windows candidate 执行上述完整 Self-Evolution 代表集；macOS arm64 与 Linux x64 Desktop candidate 至少各完成 authored JS/TS Project → dependency/Build → Candidate Test → manual Apply → next-demand invoke → Restore，以及 non-resident/idle auto-Apply smoke；required signed RC 三格都必须使用正式产品 Build 一次 authored Plugin Candidate、Apply、调用并 Restore，不能只导入预制 fixture 冒充一站式开发已交付。AP-0～AP-7 的 Agent/非 Agent consumer admission 在进入 N1-0 前先完成一次，不在每个平台重复建设第二套 Agent-only Gate。

SQLite authorizer/边界完整 suite 只在 canonical integration 环境跑一次；其他平台只做 open/query/write/migration/package smoke。

## 8. 完成定义

### 8.1 “Phase N1 功能完成”

- 05 §15 的 AP-0～AP-7 已全部通过，且 AgentPreset 的 API、Revision、Snapshot、Binding 主链不依赖本计划中的旧 Preset 形态；
- Agent 工作台以公共 `/agent` 为唯一产品路由，`/presets`、`/settings/agent-presets` 和 `/settings/agent` 不再作为长期入口；`/settings/execution-engines` 只承载 Runtime Manager；
- 用户只安装 NomiFun 即可在 Chat Dev Mode 创建 JS/TS Plugin Project、由 Agent编辑/解析依赖/Build/Test/Apply/Export；外部 JS/TS 工具链产出的合同兼容 prebuilt Artifact 也能导入同一 Test/Apply 主链；
- Runtime Manager 可复用兼容外部 Node，或经用户确认下载官方 LTS；
- Plugin 可安装、配置、调用、替换、卸载和删除保留数据；
- 五类 Contribution 进入平台 Capability Catalog，并至少被一个 Agent consumer 和一个非 Agent consumer 使用或验证，均显示 owner/provenance/availability；
- Plugin 可以在自己的 `dataDir` 使用 raw fs/SQLite，但没有核心 DB/他人 namespace 正式访问路径；
- shared Extension Host 可以整棵回收并恢复；
- Agent 可以在明确 Project scope 内修改 Plugin Source、Build 单 Ready Candidate、用 transient Test Host验证并查看 impact；manual Apply 与用户授权的 compatible-when-idle Apply 都复用现有 Replace/current/previous，busy 不打断调用，Breaking 永远人工，Agent 不获得 Package owner 或部署授权；
- Plugin Share Bundle 默认携带 Source/lock/Artifact/Test/provenance且不携带用户数据或 Credential；导入后可使用和继续开发，runtime-only Artifact 也可导入但没有 Source 时不可假装编辑；
- Headless CLI 能完成同一生命周期；
- `agent_preset_templates`（如保留）仅包含官方创建种子，不存在 Package-owned template/source 分支；
- 不存在 `contributes.presets`、`ExtPreset`、`ResolvedPreset`、`/api/presets`、`/presets` 或 Agent-only Capability Catalog 的生产可达路径。

### 8.2 “MiniApp M1 功能完成”

- MiniApp 与 Plugin 是两个清晰产品入口；
- UI-only 不启动 Node，UI+Service 只有一个 dedicated Host；
- Build 生成 immutable Release；默认人工 Publish，用户授权的 strict UI-only 变化可自动原子切换 Release 与 Catalog；
- MiniApp Active Release 的 Capability 先进入平台 Catalog，再按消费者合同供 UI、Agent、Gateway/Remote、Automation 或其他正式入口使用；
- on-demand/continuous、Bridge、KV/Files/DB、Migration、Rollback 可用；
- Trash、永久删除、Export、Import-as-new 可恢复或幂等完成；
- 带 Source 的 MiniApp Share Bundle 可正式导入、使用并继续开发；source-less 外部 prebuilt Release 可正式导入、Test/Publish 和使用，但必须导入 Source 或创建 editable fork 后才能继续开发；两者均与携带业务数据的 disabled Whole-App Backup 分开；
- 旧 MiniApp 数据没有进入新主链。

### 8.3 “内部 QA 可交付”与“Stable”

- 功能完成不等于内部 QA 可交付；后者需要 Candidate Gate、已知问题和真实安装制品；
- 内部 QA 可交付不等于 Stable；首发 Stable 需要三个 required cells（Windows x64、macOS arm64、Linux x64 Desktop）的 signed RC 证据、同 cohort、同 digest 提升和公开发布动作完成；
- 静态检查、历史包或单平台成功不能证明跨平台发布完成。
- macOS x64、Linux x64 Headless 未关闭 optional Gate 时必须标记未交付，不影响首发 Stable，也不能出现在首发支持矩阵中。

## 9. 简单性 Gate

任何新增设计进入首版前必须同时回答：

1. 它解决的是常见产品场景，还是低概率假设？
2. 删除它是否会破坏数据、凭据、发布一致性、进程清理或 required 平台交付？
3. 是否已有更小的 DB transaction、owner namespace、process kill 或显式用户动作可以解决？
4. 它是否增加新的持久化状态、恢复分支、UI 状态和跨平台测试乘积？
5. 如果后置，未来能否以独立合同增加，而不破坏当前 Package/Release identity？
6. 它是否错误地把平台级 Plugin/MiniApp Capability 绑定成 AgentPreset 专属，或重新引入第二个“设定”对象？

若第 2 题为否、且第 4 题为是，默认后置。简单不能损失正常产品体验和技术合理性；但不为极低频边界付出成倍的开发、测试和维护成本。

## 10. 方向性成本影响

以下百分比只是在加入第 7 组之前，六组简单性审计相对早期长篇设计的历史方向参考，不是当前七组范围的 ROM：

- N1 专用 conformance/fault fixture 减少约 25%～40%；
- 原生设备重复执行时间减少约 40%～60%；
- MiniApp Service identity/capacity 后端与测试减少约 20%～30%；
- MiniApp Operation/Backup/UI/CLI/fault 工作减少约 30%～45%；
- MiniApp M1 整体实现与验证有机会缩短约 20%～35%。

Plugin Self-Evolution 新增 N1 的 Chat Dev、Project Source、Build、Candidate Test、Ready/Apply、Share/Import、Workshop/CLI 和 Gate；同时把共享 JavaScript Authoring & Build Foundation 从 M1 前移到 N1，使 M1 的边际 Build 成本下降。二者不能用旧百分比直接相加减。当前不新增伪精确数字；完成 repo-level dependency scan 和 N1-0 机器合同冻结后，再按 N1-0～N1-5、M1-0～M1-2 重新给出 bottom-up 人日和关键路径。

## 11. 七组决策闭合登记

| 顺序 | 决策组 | 确认结果 | 当前状态 |
|---:|---|---|---|
| 0 | AgentPreset 前置边界 | **以 05 §15 为准：**单一 Agent 工作台、四种创建模板、平台级 Capability Catalog、多消费者、ContributionLock、旧 `/api/presets` clean cut | 已通过并签署；N1-0 已获准实施 |
| 1 | Plugin 数据与 Credential | **全部确认：**受管 KV + stable dataDir + raw Node data；Credential 下一次 resolve 生效 | 已闭合，无剩余产品裁决 |
| 2 | Package/Replace/Uninstall | **全部确认：**本地显式 Replace、用户 Retry/Restore、显式 retained-data Restore、stable contract binding、显式 breaking Replace | 已闭合，无剩余产品裁决 |
| 3 | Runtime/Host | **全部确认：**薄发现、无 Node 可静态安装、全局 Runtime 用户裁决、Host/Mount demand-load、故障后 demand-restart | 已闭合，无剩余产品裁决 |
| 4 | MiniApp Build/Release/Publish | **全部确认：**固定 Build、单 Release、纯 UI opt-in auto、单 Host cutover、Ready/Active/Previous | 已闭合，无剩余产品裁决 |
| 5 | MiniApp Service/Data | **全部确认：**run key、固定 lifecycle、capacity、Bridge、managed storage、窄 SQL/Migration、隔离受管状态 Test | 已闭合，无剩余产品裁决 |
| 6 | Operation/Gate | **全部确认：**四类 Operation、单 deleting intent、精简完整产品入口、required 三平台分层 Gate、两个最低优先 optional cells | 已闭合，无剩余产品裁决 |
| 7 | Plugin Self-Evolution | **全部确认：**Chat Dev 为默认一站式开发、外部 prebuilt Artifact 正式导入、Managed Plugin Project、共享 Build Foundation、single Ready/Test Candidate、默认人工 Apply、用户授权 compatible-when-idle、Breaking/unknown 转人工 | 已闭合，无剩余产品裁决 |

七组 Plugin/MiniApp 产品与架构合同已经形成设计基线，AP-0～AP-7 与一期 Windows C8
已经通过，N1-0 现已进入实施。下一步执行 repo-level dependency scan，冻结字段名、
Schema、IPC、默认常量和 Windows Gate；macOS/Linux required 原生验证统一延后到
Windows N1/M1 全部完成并冻结最终 cohort 之后。macOS x64 与 Linux x64 Headless
只保留 optional backlog，不进入首发关键路径。

## 2026-09-08 M1-0-02-B 实现收口与 M1-1-01 进入条件

M1-0-02-B 已在 Windows Desktop 主机完成 UI-only 生产写集，且没有把内存 Service
foundation 冒充真实 Service Host：

- Ready → Manual Publish → Active/Previous → Surface → Rollback 的 owner/pointer/Catalog
  原子切换已接入 SQLite transaction 和 exact CAS；Release identity 使用 release_id，
  content-addressed Artifact digest 可被多个 Release lineage 复用；
- auto Publish 只接受用户授权、同一 Project/source/build generation、非 UI manifest
  完全一致且已完成静态验证的严格 UI 变化；running Build、Config/Credential 变更和
  任何未知差异都会阻断；
- Surface signer 收敛为 local-trust POST /surface/open；Host 只持有 capability digest，
  iframe 先完成 nonce challenge/handshake 再接收一次性 MessagePort；Release epoch、
  session generation、active digest 和 owner 均由 Host 绑定；
- Close 是可恢复动作：后端确认成功后才卸载 descriptor，失败保留当前 descriptor 以便
  重试；KV Delete 保留 tombstone，key generation/revision 单调递增，旧 CAS 不能复活；
- Artifact/Release record 在 SQLite 和文件 Store 两侧都执行 typed validation、字段一致性、
  canonical JSON 与 expected identity 检查；固定 bootstrap 在 Build 阶段注入，Source 原始
  内容仍可继续编辑。

已通过 DB、Platform、App 和 UI 定向验证；完整命令与结果记录在本台账的 B 收口记录中。

`M1-1-01` 与 `M1-1-02` 已在 Windows Desktop 主机完成实现，设计合同保持不变：

- 一个 Service MiniApp 只绑定一个 dedicated Node Host；Host 使用私有 NDJSON IPC，
  精确校验 Hello、Runtime、Release、module digest、run key、epoch 和 generation，
  并实现 on-demand/continuous、容量、idle reap、crash backoff、Retry、EOF/timeout/
  迟到响应拒绝和 process-tree cleanup；
- Surface 与 Service 调用仍由 Host 绑定 owner、Active Release、epoch、Surface Session
  和 storage descriptor，HTTP/renderer wire 不携带路径、database path、token 或
  process identity；
- 生产 Storage 使用 owner-scoped Files directory、主 M1 DB 中的 KV tombstone/CAS 和
  独立 Host-managed Private SQLite。Private SQLite 只开放参数化 `query`、`execute`
  和最多 64 条 DML `batch`，SQLite authorizer 拒绝 Attach/Detach/PRAGMA/DDL/trigger/
  view/系统表/非主数据库；
- Migration 是 immutable ID+digest ledger；Publish 时停止旧 Service，在一个 SQLite
  transaction 内执行 CREATE TABLE/INDEX 或 ADD COLUMN，重新解析 ledger-bound Storage
  descriptor 并启动目标 Service 后才提交 Active/Previous/Catalog pointer。失败时旧
  Active 保持不变，已提交的 additive schema 不做反向回滚；
- Node `context.storage` 通过同一私有 IPC 回调 Host，提供 KV、`filesDir` 和隐藏 DB
  API；Storage callback 不阻塞 Service actor，并继承 parent invocation cancellation；
- Runtime switch participant 已对 enabled Service 逐项使用 candidate Node 做真实启动/
  Hello/停止验证，不再把生产 MiniApp Service 标记为 `NotCovered`。

实现证据已记录在 `PHASE-N1-M1-CLOSURE-TODO.zh.md` 的
“M1-1-01 / M1-1-02 实现收口”中。M1-1-02 的 SQLite/Node Storage 定向 suite 只需在
canonical Windows integration 环境运行一次；其他平台只执行自身的 open/query/write/
migration/package smoke。

当前不提前领取 M1-2：Service Test 的临时 KV/DB/files namespace 与 receipt、Trash/
Restore/Permanent Delete、Share/Backup Import-as-new、最终 Windows NSIS Candidate 及
macOS/Linux 原生验证仍按依赖顺序保留。

## 2026-09-08 M1-2 生命周期删除实现落地注记

`86afa7af6` 已按 §4.6B 的单 intent、无逐 Store phase 方案完成生命周期删除子切片：

- Trash 以 owner/pointer exact CAS 撤销 Catalog 与 Surface，并停止对应 Service；
  Restore 只恢复为 `disabled`；
- Permanent Delete 使用不可取消的 durable Operation，从头幂等清理 Service、受管
  Storage、Source、Release 和数据库 owner rows；失败保留 intent 和 Operation，由用户
  Retry 或启动 Reconciler继续；
- Source/Release/Files/Private SQLite purge 在 Windows 逐级拒绝 symlink、junction 和
  reparse point，重启后不依赖内存 registration；
- Desktop Workshop 已提供 Trash、Restore、Delete、Retry Delete 的确认、进度和错误恢复，
  不暴露 Store phase、路径、database handle 或 process identity。

该实现没有扩展 Durable Operation 集合，也没有新增 deletion journal。`M1-2` 仍未关闭：
Service Test 的 transient namespace/receipt、Share Bundle/prebuilt Import 和 disabled
Whole-App Backup Import-as-new 继续作为后续切片；Windows Candidate 与 macOS/Linux 原生
验证状态不变。

## 2026-09-09 M1-2 Service Test 实现落地注记

`708ef83b7` 已按 §4.5G 完成 transient Service Test：生产 Service 短暂停止，一次性
Storage 复制 KV/Private SQLite、创建空 Files 目录并执行 Ready Migration；独立
`MiniappServiceTestHost` Node 进程结束后整代回收，测试状态幂等删除，再恢复 Active
Service。Host-issued receipt 精确绑定 Ready、prospective epoch 生成的 service run key、
Runtime、测试输入 digest 和复制数据摘要。

该实现不把“可加载”统一冒充行为通过：无正式 callable contribution 时启动验证可为
`passed`；缺少受管输入时为 `needs_test_input`；Host 失败为 `failed`。Runtime、Ready、
Product、Config 或 Credential 漂移后 receipt 显式 `stale`。Service Test 仍不是 Durable
Operation，不创建 Test Deployment 或生产 Files overlay。M1-2 剩余 Share/prebuilt/
Whole-App Backup Import-as-new。

## 2026-09-09 M1-2 Share Application 实现落地注记

`a1534ad09`、`47f03ae0f` 已完成 Share Application/API/E2E：默认无用户数据的
Share Bundle Export/Import 与 source-less prebuilt Release Import-as-new 均已进入
MiniApp application service，并由 E2E 验证。

导入始终创建新的 disabled MiniApp identity；带 Source 的 Share Bundle 创建可继续
开发的 editable Project，source-less Artifact 保持 runtime-only，不从运行产物反推
可编辑 Source。Desktop Transfer UI 和 disabled Whole-App Backup
Export/Import-as-new 已完成，`M1-2` 的产品实现已收口。

## 2026-09-09 M1-2 Desktop Transfer 与 Whole-App Backup 实现落地注记

Desktop Transfer UI 已完成 Share Bundle、source-less prebuilt Artifact 和
Whole-App Backup 的目录选择、摘要校验、导入/导出提交与成功/失败反馈。普通用户只
选择目录和显示名称，不接触 digest、handle、generation 或内部路径。

Whole-App Backup 已完成 Windows application/API/存储闭环：固定目录格式、
canonical JSON、严格 inventory/digest/大小限制、Windows reparse/junction 拒绝；
UI-only 与 Service（Files、Private SQLite、Migration ledger）均有 application
roundtrip。导入始终创建新的 disabled identity，Active Catalog digest 按新 identity
重建，Credential slot 必须与保留 Release union 一致，ledger 不能引用未保留 Release。
Files 与 Private SQLite 使用 sibling staging + quarantine 原子替换，普通 Share Export
与 Backup Export 使用同一 owner operation 互斥。

定向证据：Backup filesystem 4、Backup application 2、Service Backup application 1、
managed storage/service-test/share 回归 7、MiniApp UI/wire 35；受影响 crate
`cargo check`、定向 rustfmt、`check:i18n` 和 UI production build 通过。`M1-2-01`
现已关闭；`M1-U-01` 的 Desktop accessibility/产品走查、`M1-V-01` 的 Windows
Candidate/NSIS/fault 验证，以及 Catalog consumer 和旧 MiniApp 清理仍按顺序进行。

## 2026-09-09 MiniApp shared Catalog publication 实现边界

MiniApp Active Release 已开始进入平台共享 Formal Capability Catalog，但本节只记录
publication/read-side 一致性：publication 使用 owner-scoped 版本更新和 tombstone；
`catalog_digest` 覆盖排序后的完整 publication；MiniApp capability 合同 owner 仍是
Package，产品生命周期 owner 仍由 MiniApp application service 管理；真实 Agent/Gateway
adapter 接入前 Agent/Gateway consumer availability 保持 unavailable；已有 Service
adapter 才标记 MiniAppService active，避免 metadata-only success。
Agent dispatch、Gateway invoke、typed resource binding、旧 MiniApp/Extension 清理和
Windows Candidate 仍按台账推进。

MiniApp Agent consumer 的第一阶段 adapter 已在 application service 内形成独立
`MiniAppAgentCapabilityPort`：它复用 Active Release、Service spec 和 dedicated Host，
并对 owner、epoch、artifact/publication digest、consumer/action 和 typed resource
做调用前校验。

## 2026-09-09 MiniApp Agent exact lock 与 Nomi Tool session 实施注记

本轮把 MiniApp capability 从“已发布但尚未被 Agent 消费”推进到第一阶段的真实
Nomi host seam，仍保持简单边界：

```text
Active Release publication
  → shared Formal Capability Catalog
  → ResolvedMiniAppCapability exact Snapshot projection
  → same Nomi dynamic Tool session as ordinary Plugin actions
  → MiniAppAgentCapabilityPort
  → dedicated Service Host
```

### A. Snapshot/Compiler

`ResolvedMiniAppCapability` 是独立的 exact value，不是 Plugin mount 的别名。它只
冻结执行所需的 MiniApp ID、Package contribution lock、Active Release/epoch、
publication/catalog digest、action descriptors、action allowlist 和 required
resource kinds；具体资源实例仍由消费目标绑定。Kernel Compiler 接收由 Control
Plane 从 Catalog 生成的 projection，拒绝重复来源、Plugin ID 冲突、Revision lock
漂移、action 越权和没有 exact placement 的能力。没有 MiniApp 时，Plugin-only
runtime profile digest 的 canonical 形状不变。

### B. Nomi session

现有 `NomiPluginToolSession` 作为当前 Nomi host 的最小 hosted Tool 容器，同时承载
Plugin 与 MiniApp action；不新增第三套 Registry。MiniApp 使用独立 `miniapp__`
provider name、schema resolver 和 invoker，ToolSearch、initial/on-demand 和
engine-owned operation identity 与 Plugin 共用同一 Nomi ToolRegistry。schema resolver
每次从 owner-scoped Release Store 校验 Active Release、epoch、catalog digest、
Package/contribution/action facts；invoker 只把 exact request 交给
`MiniAppAgentCapabilityPort`，不读取 latest Catalog、不复用 Surface Bridge。

### C. 明确未交付边界

- typed resource binding 仍 fail closed 为
  `CAPABILITY_RESOURCE_BINDING_UNAVAILABLE`，本轮不预建 MiniApp 资源 picker 或
  第二套资源数据库；
- Gateway/Remote/Automation consumer 仍保持 unavailable；
- 真实 callable MiniApp contribution 的完整产品 E2E、旧 MiniApp/Extension 生产链
 物理清理、Desktop accessibility、NSIS/Windows Candidate 和跨平台原生验证仍按
  `PHASE-N1-M1-CLOSURE-TODO.zh.md` 顺序执行；
- 不把该实现解释为 M1-U-01/M1-V-01/N1-V-01/RC-WIN-01 已关闭。

## 2026-09-09 Guid 启动入口与模型冻结实施注记

Guid 的 Agent 选择器现在把普通 Nomi 与 AgentPreset 分成两条清晰路径：

- `default` 选择显示模型选择器，并沿用普通 Conversation create；
- `preset/template` 选择不显示模型选择器，也不从客户端向 Agent Session Create
  提交模型、route、binding 或 snapshot；服务端使用稳定 Revision/Snapshot；
- 官方 template preparation 的 `model` 变为可选，缺省时由 Control Plane 解析 default
  Chat route；保存后的 AgentPreset Session 继续由 Snapshot 冻结模型；
- AutoWork/发送按钮的 launch target 判断按当前模式分别校验，不再因为普通模型缓存
  缺失而错误禁用已保存 AgentPreset。

该修正只收敛产品入口，不增加持久化状态或兼容层；`gate:agent-v2 --self-test`、
Guid 定向测试和 UI production build 通过。
