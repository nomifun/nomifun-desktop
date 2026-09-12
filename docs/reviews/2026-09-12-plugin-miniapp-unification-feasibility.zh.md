# Plugin 与 MiniApp 统一为“一切皆插件”的可行性调研报告

- 调研日期：2026-09-12
- 调研对象：`nomifun-desktop` 重构后的 Plugin 与 MiniApp（小程序）主链
- 调研性质：代码、契约、数据模型、运行时、路由、CLI、前端产品形态和设计文档的交叉审查
- 结论等级：**建议实施，但必须采用分层统一和渐进迁移；不建议直接把两套运行时和数据表粗暴合并**

## 1. 执行摘要

本次调研对用户提出的方向作如下判断：

> 对外可以统一成一个“Plugin/插件”概念。插件可以只有系统能力注入，也可以有自己的产品形态；可以是纯 UI，可以是后台 Service，也可以是 UI + Service 的全栈应用。MiniApp 不再作为独立产品概念和一级入口存在，而成为 Plugin 的一种内部形态或角色组合。

这个方向总体合理，而且与当前代码已经形成的若干事实相吻合：

- Plugin 当前已经不是单纯的无头能力包，前端已有完整的 Library、Workshop、AI Creator、Detail、Build、Test、Apply、Configure、Share 和安装生命周期工作台。
- Plugin 与 MiniApp 已经共享 JavaScript 构建基础、Capability Catalog、Runtime Manager、Node 进程基础设施、owner/CAS/digest/provenance 等大量平台抽象。
- MiniApp 的能力已经进入共享 Capability Catalog，并且其 Agent action 已经可以通过与 Plugin 共用的 Nomi 动态 Tool session 暴露给消费者。
- 两套系统已经在应用组合根、Runtime Manager 和消费者入口处发生汇合，因此继续保持两个完全平行的产品概念，会造成真实的产品重复和维护重复。

但以下判断同样重要：

- MiniApp **不是**“只比 Plugin 多一个 UI 开关”。
- MiniApp 还拥有独立的 Product/Project/Release 聚合、Active Release epoch、Surface session、Host-owned MessageChannel、Dedicated Service Host、`service_run_key`、受管 Files/Private SQLite、Migration ledger、Service Test receipt、Whole-App Backup 和整产品删除恢复语义。
- Plugin 当前的 `plugin-package-v1` 运行时仍然明确限制为 `main.mjs` capability provider，不能发布 Plugin Service、Service DAG、Role Provider，也没有 MiniApp 的长驻 Service 和 Surface 生命周期。
- 因此，**产品概念和平台领域内核可以统一；运行时角色和协议不能为了概念统一而强行变成同一种协议**。

最终建议是：

1. 立即采用“Plugin 是总概念”的产品方向；
2. 统一产品入口、Library/Workshop 投影和公共领域模型；
3. 将 MiniApp 降级为内部 `Surface`、`Service`、`Full-stack` 等角色组合，而不是继续作为用户可见的产品分类；
4. 保留 Shared Extension Host、Dedicated Service Host、Surface Bridge、Build Host 等不同运行时角色；
5. 通过兼容适配、影子投影和可回滚迁移逐步退出 `/mini-apps`、`/api/miniapps` 和 `miniapp` CLI 名称；
6. 暂不直接合并数据库表、直接把 MiniApp Release 当作 Plugin Package，或让普通 Plugin Shared Host 承担长驻 Service。

一句话结论：

> **“一切皆插件”作为产品和平台方向是正确的；“一切运行时都按同一种 Plugin Host 执行”则不符合当前实现事实，也不是必要条件。**

## 2. 对用户设想的准确化

用户补充后的想法不是“把 MiniApp 变成带 UI 的 Plugin”，而是更完整的统一模型：

| 维度 | 统一后的 Plugin 可以是什么 |
| --- | --- |
| 产品形态 | 没有独立页面的系统能力、嵌入式 UI、独立应用页面、后台自动化、全栈应用 |
| 执行形态 | 按需调用、驻留 Service、定时/事件驱动、UI Surface、UI + Service 组合 |
| 能力消费者 | Agent、Gateway/Remote、UI、Automation、Knowledge、其他 Plugin/Service |
| 数据能力 | 无持久化、Host KV、受管 Files、Private SQLite、插件自己的数据目录 |
| 开发交付 | NomiFun 内部 Chat Dev、外部工具链构建的 prebuilt Artifact、导入/分享 |
| 生命周期 | 创建、构建、测试、候选、启用、发布、运行、停用、回滚、卸载、删除 |

这里需要把三个概念层次分开：

### 2.1 产品概念层

对用户只保留一个入口和一个总称，例如 `Plugins` 或 `Extensions`。用户创建的是“插件”，创建过程中选择或描述它需要的产品能力，而不是先选择“Plugin”还是“MiniApp”。

### 2.2 平台领域模型层

所有插件都应尽量使用同一套抽象：

- identity 和 owner；
- Project/Source lineage；
- Build operation；
- immutable Artifact/Release；
- digest、provenance 和 test receipt；
- Candidate、Ready、Current/Active、Previous；
- Config/Credential；
- Catalog publication；
- revision/CAS；
- import/export/share；
- durable operation 和启动恢复。

### 2.3 运行时角色层

不同插件可以采用不同的执行拓扑：

- 多个短调用能力共享一个 Extension Host；
- 一个需要隔离和长期运行的 Service 独占一个 Dedicated Service Host；
- UI 通过 Surface Host 和 Host-owned MessageChannel 运行；
- Build 和 Candidate Test 使用一次性的 Build/Test Host。

这三个层次不能互相替代。产品概念统一，不代表每一种执行形态必须共用相同的进程、IPC 消息和状态机。

## 3. 调研范围与方法

本次调研覆盖以下层面：

1. Phase N1/M1 设计与实施文档；
2. Rust Agent contracts、Package/Capability/Catalog 契约；
3. Plugin Platform、JavaScript Kernel Adapter、Shared Extension Host；
4. MiniApp Platform 的 model、release、runtime、service host、service process、bridge、storage、share 和 operation；
5. SQLite migrations、repository 和 CAS/trigger 约束；
6. `AppServices`、Runtime Manager、路由组合根和生命周期关闭路径；
7. HTTP API、CLI、React Router、侧栏和两个产品的前端工作台；
8. Git 历史中 N1 Plugin 与 M1 MiniApp 的演进方式；
9. `PHASE-N1-M1-CLOSURE-TODO.zh.md` 中的完成状态与剩余边界。

报告中的代码证据使用“文件路径:行号”表示。行号来自本次调研时的工作树，后续代码改动可能使行号发生偏移，但结构和类型名称仍可用于定位。

本次只新增本报告文件，没有修改现有业务代码，也没有把工作区已有的用户修改纳入本次变更。

## 4. 当前总体架构：两条产品主链，共享多个平台层

早期实施计划明确将两者设计成：

> 两个并列的平台产品和一套共享技术底座。

设计文档在 `docs/specs/2026-08-28-agent-capability-platform-v2/06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md:298-307` 中给出了原始边界：

- Plugin：扩展 NomiFun 平台能力的组件；
- MiniApp：用户可以打开、使用和持续运行的应用；
- 两者共享 JavaScript Host Foundation、Runtime Manager、Service Host 和公共协议基础设施；
- 但当时明确保留不同的顶层身份、Manifest、产品入口、发布生命周期和删除语义。

当前代码已经形成如下结构：

```text
                         ┌──────────────────────────────┐
                         │  Shared Platform Foundations  │
                         │  Runtime / Build / Catalog    │
                         │  owner / CAS / digest        │
                         └──────────────┬───────────────┘
                                        │
              ┌─────────────────────────┴─────────────────────────┐
              │                                                   │
   ┌──────────▼──────────┐                            ┌───────────▼───────────┐
   │ Plugin product chain │                            │ MiniApp product chain │
   │ Project / Package    │                            │ Product / Project      │
   │ Candidate / Mount    │                            │ Release / Active      │
   │ Shared Host          │                            │ Surface / Service     │
   └──────────┬──────────┘                            └───────────┬───────────┘
              │                                                   │
              └─────────────────┬─────────────────────────────────┘
                                │
                    ┌───────────▼───────────┐
                    │ Shared Capability      │
                    │ Catalog / Consumers    │
                    └───────────────────────┘
```

这个架构说明两件事：

1. 现在的重叠不是错觉，公共基础已经很大；
2. 现在的差异也不是单纯命名差异，两条产品主链仍有各自不可忽略的运行时和数据不变量。

此外，设计文档的早期产品口径已经落后于当前实现。文档仍把 Plugin 描述为“无独立产品 UI”，但当前 `ui/src/renderer/pages/plugins/PluginWorkbenchPage.tsx:72-109、625-889` 已经有：

- `home | creator | detail` 三种产品面；
- Library 与 Workshop 两个工作区；
- AI 创建和持续编辑；
- Source、Dependency、Build、Candidate Test、Apply、Share；
- Mount Configure、Enable/Disable、Retry、Restore、Uninstall、Delete Data；
- Project 与已安装 Mount 的统一产品投影。

因此，当前评估必须以“实现已经向产品化 Plugin 演进”为前提，而不能只复述旧设计中的二元产品边界。

## 5. Plugin 当前设计与实现现状

### 5.1 Plugin 已经具备产品形态

当前 Plugin 不是单纯的后台 SDK。其工作台已经承担了完整的产品管理职责：

- Library：展示已安装和可使用的 Plugin；
- Workshop：管理作者 Project、Source、依赖和 Ready Candidate；
- Creator：通过 AI/Chat Dev 生成或修改项目；
- Detail：查看能力、消费者、运行状态、配置和数据；
- 生命周期：Enable、Disable、Retry、Restore、Uninstall、Delete Data；
- 交付：Build、Candidate Test、Apply、Share、Import。

这与用户补充的“插件可以有产品形态，也可以只向系统注入能力”是相容的。区别只在于当前 Plugin 的产品形态主要是“平台提供的管理工作台”，还不是 Plugin 自己通过 Surface 提供的业务应用 UI。

### 5.2 Plugin 的 Manifest/Build profile 仍然比较窄

`crates/backend/nomifun-agent-contracts/src/plugin_n1.rs` 的 `JavaScriptBuildProfile` 同时列出：

- `PluginPackageV1`
- `MiniAppReleaseV1`

但 `PluginPackageV1Manifest` 在 `plugin_n1.rs:1109-1190` 明确要求：

- build profile 必须是 `plugin_package_v1`；
- JavaScript entrypoint 必须是 `main.mjs`；
- 不允许 package dependencies；
- 不允许 `provides_services` 或 `requires_services`；
- 不允许 Role Contract/Role Provider；
- Artifact 文件基本限定为 `main.mjs`、source map 和 `resources/**`。

JavaScript Kernel Adapter 还在 `crates/backend/nomifun-js-kernel-adapter/src/lib.rs:45-120` 和 `98-102` 主动拒绝：

- Plugin Service；
- Node Role Provider；
- Service DAG；
- 当前 Plugin Package 不支持的角色贡献。

当前正式注册的 Plugin capability 主要是：

- Tool；
- Context Contributor；
- Resource Provider。

这意味着：

> Plugin 的产品管理层已经足够宽泛，但 Plugin Package v1 的运行时仍是“无头 capability provider” profile。

这也是统一方向最重要的改造点之一：不是重新发明一个 MiniApp，而是扩展 Plugin 的 profile/role 表达能力。

### 5.3 Plugin 的运行时是 Shared Extension Host

`crates/backend/nomifun-js-host/src/supervisor.rs` 的 `ExtensionHostSupervisor` 具有以下特征：

- 一个 Host generation 可以承载多个 Plugin Mount；
- `load_mount`、`unload_mount`、`invoke`、Context contribution、Resource acquire/release 都在同一个 Host 体系内；
- 支持 demand-triggered lazy load；
- 每一代有 generation fence；
- 支持 request timeout、cancel、watchdog、shutdown 和进程树清理；
- 一个 generation 内的 Host/protocol failure 可能影响同一代的多个 Mount。

关键接口和状态可在 `supervisor.rs:378-395、484-585、763-920、1190-1328、1458-1605` 定位。

这是一种适合“很多小能力共享一套进程”的拓扑，但它的共同故障域也很明确。它不是为每个插件提供独立长驻进程的应用容器。

### 5.4 Plugin 的数据与安装语义

Plugin 的当前安装实例是 Mount，作者资产是 Project，两者不是同一个对象：

- Project 保存 Source、依赖锁、Build generation、Ready Candidate；
- Mount 保存当前/上一版本 Artifact、配置、enabled、retained、delete_pending、revision 和稳定 `data_dir`；
- Project 删除不会自动级联已安装 Mount；
- Uninstall 默认保留运行数据和可选作者 Project；
- Delete Data 单独清理 Config、KV、Credential binding 和 `dataDir`；
- Plugin 可以使用 raw `node:fs`、`node:sqlite` 和 pure-JS 数据库封装自己的 `dataDir`；
- NomiFun 不把核心数据库或他人的 namespace 暴露给 Plugin。

数据库证据见：

- `crates/backend/nomifun-db/migrations/067_plugin_n1_data_root.sql:8-51、199-316、431-500`；
- `plugin_n1.rs:1858-1880`；
- `plugin_n1.rs:745-762`。

Plugin 的当前生命周期更接近“安装组件/运行绑定”：

```text
Project Source
  → Build
  → Ready Candidate
  → Candidate Test
  → Apply 到 Mount
  → Current / Previous
  → Uninstall（保留数据）
  → Delete Data（显式清理）
```

这与 MiniApp 的“产品被放入回收站，再执行整产品删除”并不相同。

### 5.5 Plugin 已经与多消费者模型接轨

`ui/src/common/types/pluginPlatform.ts:40-47` 的消费者面已经包括：

- agent；
- gateway；
- knowledge；
- remote；
- automation；
- ui；
- miniapp_service。

`pluginPlatform.ts:23-31、150-216、364-456` 还暴露了：

- enabled/disabled/uninstalled-data-retained/delete-pending/error；
- Candidate、Test receipt、Impact；
- auto-apply 授权；
- Share/Import；
- MiniApp permanent delete 对应的 owner/operation 类型。

这表明 Plugin 平台本身已经不再是“只为 Agent 提供 Tool”的狭窄系统，用户提出的“插件也可以是产品形态或系统能力”与类型层的演进方向是一致的。

## 6. MiniApp 当前设计与实现现状

### 6.1 MiniApp 是完整的产品聚合，不是一个 UI 文件夹

`crates/backend/nomifun-miniapp-platform/src/model.rs:21-24、134-192、250-317` 定义了独立的 MiniApp 聚合：

- `MiniAppKind::{UiOnly, Service}`；
- `MiniAppProduct`；
- Product lifecycle；
- Project/source；
- immutable Release inventory；
- Ready、Active、Previous 指针；
- Config、Credential binding；
- Service storage；
- import provenance；
- 结构校验和 owner 隔离。

`validate_structure` 不只是检查 UI 文件是否存在，还会检查：

- identity、revision 和 owner；
- Release inventory 与指针的一致性；
- Active Release 与 `active_release_epoch` 的一致性；
- UI-only 不得持有 Files/Private DB；
- Service 类型必须存在且仅存在一个 `service/main.mjs`；
- Service 的 storage contract 必须与 managed data root 一致。

因此，MiniApp 的核心对象是“可发布、可运行、可恢复的产品”，而不是单纯的“前端页面”。

### 6.2 MiniApp Release 是 UI 与可选 Service 的不可变发布单元

`crates/backend/nomifun-agent-contracts/src/miniapp_m1.rs:412-560、604-700` 的 `MiniAppReleaseV1Manifest` 同时包含：

- UI entrypoint；
- 可选 Service descriptor；
- lifecycle（`on_demand` / `continuous`）；
- dependency lock/graph；
- Config schema；
- Credential slots；
- Resource contract；
- Bridge contract；
- capability contributions；
- migrations。

Release artifact 的有效结构是：

```text
manifest
ui/**
optional service/main.mjs
```

MiniApp 的 Release digest 覆盖完整发布单元。设计文档在 `06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md:711-749` 明确规定：

- Build 不执行任意用户 build/install/postinstall script；
- Release 是 immutable；
- Ready、Active、Previous、Publish、Rollback、Export 和 consumer provenance 都以完整 Release 为单位；
- UI 或 Service 的内部 hash 可以用于缓存，但不拆成独立产品发布单元。

### 6.3 MiniApp Service 使用 Dedicated Service Host

MiniApp 的 Service 不是把一个普通 Plugin Mount 变成长驻即可。当前实现包含独立运行时语义：

- 一个 Service MiniApp 对应一个 dedicated Node process；
- `service/main.mjs` 必须导出 `start(context)`；
- 返回对象必须提供 `invoke`，可选 `dispose`；
- 通过独立 Service NDJSON protocol 处理 invoke/cancel/shutdown；
- 启动 Hello 精确绑定 MiniApp、Release、epoch、`service_run_key`、runtime 和 Host generation；
- 支持 on-demand lazy start；
- 支持 continuous 自动启动；
- 支持 crash/backoff/retry；
- 支持 idle reap；
- 支持容量限制；
- Service 失败不会无限重启。

`crates/backend/nomifun-miniapp-platform/src/service_host.rs:18-18、38-50、101-124、227-315、343-439、492-716` 显示：

- 默认最多 `4` 个 active Service Host；
- 每个 MiniApp 独占一个 capacity slot；
- Service generation fence 绑定 MiniApp、Release、epoch、`service_run_key` 和 Host generation；
- continuous Service 使用有限失败退避并可进入 Error；
- on-demand Service 在空闲后回收。

`service_process.rs:254-358、739-750、912-950、1450-1465、1909-1965` 进一步显示：

- Service 进程在独立 Node 进程中启动；
- request watchdog、取消和 frame 限制是 Service 协议的一部分；
- storage 请求通过私有 IPC 回调 Host；
- EOF、迟到响应和 digest/fence 不匹配会 fail closed；
- 整棵 process tree 必须可清理。

这套机制和 Plugin Shared Extension Host 的“多 Mount、共同 generation、短调用”不是同一个协议。

### 6.4 MiniApp Surface 是独立的 Host-owned Bridge

MiniApp UI 运行在 Surface 中，前端 `MiniAppSurfacePanel.tsx` 使用 iframe，并完成：

- nonce challenge；
- handshake；
- MessageChannel；
- 一次性 MessagePort；
- session/generation 校验；
- bridge call/cancel/close/reload；
- sandbox 和 `no-referrer` 约束。

`crates/backend/nomifun-miniapp-platform/src/bridge.rs:17-20、42-76、130-232、249-318、355-376` 明确规定：

- Surface 不能自行选择 owner、Release、epoch 或 storage；
- Host 创建并持有 MessageChannel endpoint；
- 旧 session、旧 Release、旧 epoch 的 port 会被拒绝或关闭；
- in-flight call 可以取消；
- UI-only 只能访问 Host KV，不能访问 Service、Files 或 Private DB；
- 不开放 localhost bridge。

前端证据见 `ui/src/renderer/pages/miniApps/MiniAppSurfacePanel.tsx:45-50、166-177、202-299、310-427、520-528`。

因此 Surface 调用不能简单地当成普通 `CapabilityInvoke`：

- 普通 capability 调用的调用方是平台消费者；
- Surface Bridge 的调用方是受 Host 绑定的 Web 内容；
- 后者必须同时绑定 iframe/window、nonce、session、port、Release、epoch 和 owner。

### 6.5 MiniApp 拥有更强的受管数据能力

MiniApp 的 Service 可以使用：

- Host KV；
- owner-scoped Files directory；
- Host-managed Private SQLite；
- additive migration ledger。

Private SQLite 只通过窄 API 使用：

- 参数化 `query`；
- 参数化 `execute`；
- 有限 batch；
- 不暴露数据库路径；
- 不支持 `ATTACH`、extension、任意 PRAGMA、运行时 DDL 或用户持有 transaction。

Migration 是 Release cutover 的一部分。`runtime.rs:19-110、115-150、214-350` 显示：

- Publish/rollback 都有 prepare/complete/abort；
- target epoch 必须递增；
- Publish 需要带 target Release 的 migration 集；
- Rollback 不执行逆向 migration；
- Service spec 必须精确绑定 MiniApp、Release 和 epoch；
- UI-only runtime 永远不启动 Node。

### 6.6 MiniApp 的 Test、Backup、Share、Delete 是应用级语义

MiniApp Service Test 不是简单加载 `main.mjs`：

1. 停止生产 Service；
2. 复制 KV 和 Private SQLite 到一次性 test namespace；
3. 创建空的临时 Files 目录；
4. 执行 Ready Release 的 migration；
5. 启动 one-shot Test Host；
6. 写入 immutable Test receipt；
7. 清理测试存储；
8. 恢复生产 Service。

`miniapp_m1.rs:2058-2188` 和 `083_miniapp_service_test_receipts.sql:9-102` 中的 receipt 精确绑定：

- MiniApp；
- Release；
- Service run key；
- Runtime fingerprint；
- Host generation；
- storage digest；
- migration ledger；
- credential mode。

MiniApp Backup 还要保存 Source、Release、Config、KV、Files、Private DB 和 migration ledger，但不恢复本机 Credential binding；导入始终创建新 MiniApp identity。

删除语义是：

```text
Enabled / Disabled
  → Trashed
  → 停止 Service、撤销 Catalog 和 Surface
  → Restore 回 Disabled
  → Permanent Delete
  → durable deleting intent
  → 幂等清理 Source / Release / Config / Credential binding /
    KV / Files / Private DB / owner rows
```

`082_miniapp_deletion_intents.sql:1-43` 明确把 deletion intent 与普通 product operation 分离，以便失败后由启动 Reconciler 或用户 Retry 继续。

这些语义已经超出“UI 模式”的范围。

## 7. 两套系统的真实重叠点

重叠是结构性的，不只是目录名称相似。主要重叠如下：

| 领域 | Plugin | MiniApp | 统一价值 |
| --- | --- | --- | --- |
| Owner/权限 | owner-scoped、local-trust、安装所有者 | owner-scoped、local-trust、产品所有者 | 统一授权和边界检查 |
| 作者资产 | Project、Source snapshot、dependency lock | Project、Source snapshot、dependency lock | 统一 Authoring/Source lineage |
| 构建 | Build Operation、staging、cancel、失败清理 | Build Operation、staging、cancel、失败清理 | 统一 Build orchestration |
| 产物 | immutable Plugin Artifact | immutable MiniApp Artifact/Release | 统一 digest/provenance/CAS |
| 候选 | Ready Candidate、Test receipt、Impact | Ready Release、Service Test receipt | 统一候选和验证抽象 |
| 发布指针 | Current/Previous Mount target | Ready/Active/Previous Release | 统一 Deployment slot 抽象 |
| 配置凭据 | Config schema、Credential slot/binding | Config schema、Credential slot/binding | 统一配置和凭据引用 |
| Capability | Plugin contribution | MiniApp Release contribution | 统一 Catalog publication |
| 消费者 | Agent/Gateway/UI/Automation 等 | Agent/Gateway/UI/Automation 等 | 统一 consumer admission |
| Digest/fence | Mount、Artifact、Contribution、Host generation | Release、epoch、Service run key、Surface session | 统一 exact lock 基础设施 |
| 运行时管理 | Node Runtime、Build/Host admission | Node Runtime、Service/Surface admission | 统一 Runtime authority |
| 传输/取消 | NDJSON、request ID、cancel、watchdog | NDJSON、request ID、cancel、watchdog | 统一可靠性组件 |
| 迁移恢复 | durable operation、startup reconciliation | durable operation、deletion intent、startup reconciliation | 统一恢复框架 |
| 交付 | Share Bundle、prebuilt import | Share Bundle、prebuilt release、Whole-App Backup | 统一 transfer envelope |
| 产品工作台 | Library/Workshop/Creator/Detail | Library/Workshop/Runner/Surface | 统一产品导航和投影 |

其中有一部分已经在代码中实现为共享事实：

- `PackageManifest` 已统一包含 dependencies、runtime features、config schema、services、entrypoint 和 contributions，见 `crates/backend/nomifun-agent-contracts/src/package.rs:378-430`；
- `CapabilityKind` 已包括 Tool、ContextContributor、ResourceProvider、Event、Scheduler、BackgroundService、UiContribution；
- `CapabilityConsumer` 已包括 Agent、Gateway、Knowledge、Remote、Automation、UI、MiniAppService；
- `CapabilityProvenance` 同时支持 Plugin Mount 和 MiniApp Active Release，见 `crates/backend/nomifun-agent-contracts/src/catalog.rs:34-80`；
- `CapabilityCatalogEntry` 和 `CapabilityOperationLock` 已统一描述 capability、contract digest、provenance、consumer、availability 和 release admission，见 `catalog.rs:225-241、523-548、633-709`；
- `AppServices` 同时持有 Plugin 运行时和 `miniapp_application`，见 `crates/backend/nomifun-app/src/services.rs:1978-1981`；
- Nomi 动态 Tool session 已经同时承载 Plugin 和 MiniApp action，但使用不同 invoker 和 exact projection，见 `PHASE-N1-M1-CLOSURE-TODO.zh.md:728-738`。

这说明统一方向不是从零开始设计，而是将已有共享层向上提升为统一的 Extension 内核。

## 8. 不能直接等同的差异

以下差异决定了“直接把 MiniApp 改名成 Plugin Mount”不可行：

| 维度 | Plugin 当前语义 | MiniApp 当前语义 | 不能直接合并的原因 |
| --- | --- | --- | --- |
| 顶层身份 | Package/Project + 安装 Mount | Product/Project + Release | 一个 Package 可有多个安装绑定；MiniApp 的产品 identity 必须稳定 |
| 代码入口 | `main.mjs` | `ui/index.html` + 可选 `service/main.mjs` | UI 资源和 Service 进程的启动契约不同 |
| Host 拓扑 | 多 Mount 共用 Shared Extension Host | 一个 Service 独占 Dedicated Host | 故障域、容量、生命周期不同 |
| 长驻能力 | 当前主要是 demand invoke | on-demand/continuous、idle reap、crash backoff | 需要独立 Service 状态机 |
| UI 通道 | 当前没有同构的产品 Surface | iframe + nonce + MessageChannel + session | Web 内容边界不能当普通 capability RPC |
| Release 指针 | Mount current/previous | Ready/Active/Previous + epoch | MiniApp 发布需要 Active epoch 和 Surface/Service fence |
| 数据 | KV + stable raw `dataDir`，Plugin 自管 fs/SQLite | Host KV + managed Files + Private SQLite + migration | 存储 ownership、备份、迁移和删除边界不同 |
| 发布失败 | Apply/Restore 主要回退代码 | Service 停止、migration、启动目标、切换指针、恢复旧 Service | 需要 pre-commit 与 post-commit 两种故障语义 |
| 测试 | transient Candidate Test Host 和一次性 dataDir | Service Test storage copy、migration、receipt、恢复生产 Service | 测试副作用和证明材料不同 |
| 删除 | Uninstall 保留 Mount 数据；Delete Data 独立 | Trash/Restore/Permanent Delete 整产品清理 | 用户预期和恢复策略不同 |
| 备份 | 默认不含运行数据和 Credential | Whole-App Backup 可含业务数据，但不含本机凭据绑定 | 传输载荷和安全风险不同 |
| 消费者锁 | Mount/Artifact/Contribution lock | MiniApp/Release/epoch/Catalog digest lock | provenance 结构不同，不能用一个字段替代 |
| 安全边界 | Plugin capability provider | Surface Web 内容 + Service process | 权限授予和攻击面不同 |

最关键的三个差异是：

1. **Shared Host 与 Dedicated Host 的拓扑差异**；
2. **Surface Bridge 与 Capability Invoke 的协议差异**；
3. **Plugin raw dataDir 与 MiniApp managed storage/migration 的数据差异**。

这三个差异都不是 UI 差异。

## 9. “MiniApp 是否只是带 UI 的 Plugin”结论

### 9.1 狭义判断：不成立

如果“Plugin”指当前 `plugin-package-v1`，那么 MiniApp 不是它加一个 `has_ui=true`：

```text
当前 Plugin Package
  = main.mjs capability provider
  + Shared Extension Host Mount
  + Config/Credential/KV/dataDir

当前 MiniApp Service
  = ui/index.html
  + optional service/main.mjs
  + Active Release / epoch
  + Surface Bridge
  + Dedicated Service Host
  + managed storage / migration / test / backup / delete
```

两者的运行输入、生命周期、故障域和安全边界都不同。

### 9.2 广义判断：可以成立，但需要换一个定义

如果把 Plugin 定义为“所有可安装、可构建、可发布、可向平台或用户提供能力的 Extension”，那么更准确的关系是：

```text
Plugin / Extension
  ├─ Headless capability plugin
  ├─ UI Surface plugin
  ├─ Background Service plugin
  ├─ UI + Service full-stack plugin
  ├─ Agent/Gateway/Automation contribution plugin
  └─ 多种角色的组合
```

在这个模型中，MiniApp 不再是产品概念，只是历史名称或一种内部角色组合：

```text
MiniApp（兼容层旧名）
  ≈ Plugin
  + Surface
  + optional Dedicated Service
  + app-level data/release semantics
```

因此，用户的统一方向应表述为：

> MiniApp 不是“另一个产品种类”，而是 Plugin/Extension 可以拥有的一组产品和运行时角色。

## 10. 当前设计是否支持统一方向

结论是：**支持，而且已有较好的落点；但当前实现仍处于“两个 profile、两个 aggregate、多个 runtime role”的中间态。**

### 10.1 支持统一的证据

- Package/Catalog 契约已经有通用的 services、contributions、consumers、UI/BackgroundService 等表达能力；
- Plugin 的消费者类型已经包含 UI 和 `miniapp_service`；
- MiniApp capability 已进入共享 Catalog；
- MiniApp Agent action 已复用 Nomi Plugin Tool session 的容器；
- App 组合根已经同时接入两套系统；
- Runtime Manager 已经把 Plugin Mount、MiniApp Service、Build Foundation 作为 runtime switch participant；
- 两套系统都使用 exact digest、owner、revision、CAS 和 fail-closed admission；
- Plugin 前端已经是产品化工作台，说明产品入口统一不会从零开始。

### 10.2 暂不支持“一步到位”的证据

- `PluginPackageV1Manifest` 明确拒绝 Service 和 Role Provider；
- Plugin Shared Host 只能共同管理一代多个 Mount，缺少 MiniApp Service 的独立容量和重启状态机；
- MiniApp Release 需要 Surface/Service epoch，而 Plugin Mount 没有同构的 Active Release epoch；
- MiniApp 的 managed Files/Private SQLite/migration ledger 没有 Plugin 对应物；
- MiniApp DB migration 明确独立于 Plugin，且 build lineage 也单独保存；
- 两套 API、CLI、路由、删除和备份合同仍是分开的；
- MiniApp 的 Surface Session 和 Bridge port 不能直接放进普通 Plugin capability invoke；
- 当前设计文档仍把“Plugin 与 MiniApp 不合并”写成产品合同，表明迁移需要一次明确的架构决策，而不是隐式改名。

## 11. 三种方案比较

### 方案 A：继续保留两个产品，只共享底座

做法：

- 保留 Plugin 与 MiniApp 两个一级入口；
- 继续分别维护 Product/Project/Release/Mount；
- 只抽取 Build、Catalog、Runtime 和公共工具。

优点：

- 现有代码改动最小；
- 不需要马上处理身份和数据库迁移；
- 运行时边界清晰。

缺点：

- 用户仍需理解两套概念；
- Library、Creator、Build、Test、Share、生命周期等功能继续重复；
- 后续新增 UI/Service/Background/Automation 组合时，容易再次出现第三套产品；
- 公共领域模型继续通过适配层重复表达；
- 产品文案和导航复杂度持续存在。

评价：适合短期保守维护，不适合用户提出的统一产品方向。

### 方案 B：统一产品入口和领域内核，内部保留运行时角色

做法：

- 对外只显示 Plugin/Extension；
- MiniApp 变成 Plugin 的 Surface/Service/Full-stack 角色组合；
- 建立统一 Extension 投影和公共生命周期；
- 保留 Shared Extension Host、Dedicated Service Host、Surface Host、Build/Test Host；
- 旧 `/mini-apps`、`/api/miniapps`、`miniapp` CLI 先做兼容别名和迁移；
- 逐步把两套 profile 适配到统一的 Extension Manifest envelope。

优点：

- 产品概念明显简化；
- 可以复用当前已存在的公共 Catalog、Runtime、Build 和 owner/CAS 基础；
- 不牺牲 Service、Surface、storage、migration 的安全边界；
- 风险可分阶段控制；
- 未来可以自然加入“纯 UI”“纯 Service”“全栈”等组合。

缺点：

- 需要新增 canonical identity/projection；
- 需要迁移 API、前端、CLI、文案、数据库读取模型；
- 一段时间内会同时维护旧名和新名；
- 内部 role 类型会比当前两个顶层名称更丰富。

评价：**推荐方案。**

### 方案 C：彻底统一 Extension domain 和单一运行时协议

做法：

- 将 Plugin Mount、MiniApp Product、Release、Surface、Service 全部强行变成一种 Extension；
- 一个 Host protocol 处理 Mount、UI、Service、Storage、Bridge 和长驻生命周期；
- 删除两套旧模型和所有 role-specific contract。

优点：

- 理论上概念和代码数量最少；
- 长期可以获得一个极简的 API 表面。

缺点：

- 会把进程拓扑、故障域、容量、Surface 安全、migration、storage 和调用语义混成一个复杂大协议；
- Shared Host 与 Dedicated Host 的隔离目标互相冲突；
- 需要一次性迁移大量持久化数据和运行状态；
- 很容易为了“统一”牺牲 fail-closed、安全和可恢复性；
- 单一协议会出现大量可选字段和状态组合，实际复杂度可能高于当前多角色协议；
- 迁移失败时很难回滚到当前稳定链。

评价：不建议作为当前重构动作；可以把“统一 Supervisor 外壳”和“统一 Manifest envelope”作为长期目标，但不应把所有运行时实现压成一个协议。

## 12. 推荐目标架构

### 12.1 对外名称与内部名称

建议：

- 对外产品名称：`Plugin` 或 `Plugins`；
- 内部平台领域名称：`Extension`；
- `MiniApp` 作为兼容期的 legacy alias 和内部迁移标签；
- 代码中的 `MiniApp` 模块可以先保留，直到数据、API、测试和安装制品完成迁移。

使用内部 `Extension` 名称有一个现实好处：当前 `Plugin` 在代码里同时指 Package、Project、Mount 和产品入口，直接把所有对象都命名为 Plugin 容易产生新的歧义。

### 12.2 不要把所有形态压成一个枚举

建议把形态拆成正交维度，而不是建立一个互斥的：

```text
kind = headless | miniapp | fullstack
```

可以采用类似的内部模型：

```text
Extension
  identity
  owner
  project/source
  artifact/release
  contributions[]
  surfaces[]
  services[]
  storage_profile
  consumer_bindings[]
  lifecycle
  provenance
```

其中：

| 正交维度 | 建议值 |
| --- | --- |
| Surface | none、embedded、standalone、multiple |
| Capability contribution | Tool、Context、Resource、Event、Scheduler、UI 等 |
| Service | none、on-demand、continuous、dedicated |
| Storage | none、Host KV、managed Files、Private DB、plugin dataDir |
| Consumer | Agent、Gateway、UI、Automation、Remote 等 |
| Distribution | authored source、prebuilt artifact、share、backup |
| Lifecycle | draft、ready、active、disabled、trashed、deleting、error |

这样同一个 Plugin 可以同时拥有 UI、Service、Tool 和 Automation contribution，而不需要再创建“Plugin Plus”“MiniApp Pro”之类的第二层产品分类。

### 12.3 统一 Extension Manifest envelope

建议最终引入一个上层 envelope，概念上类似：

```text
ExtensionManifest
  ├─ identity / display / version
  ├─ build profile
  ├─ contributions
  ├─ supported consumers
  ├─ ui?: SurfaceDescriptor
  ├─ services?: ServiceDescriptor[]
  ├─ storage?: StorageProfile
  ├─ config / credentials
  ├─ migrations
  └─ lifecycle / admission policies
```

但迁移初期不要破坏现有合同：

```text
ExtensionManifest
  ├─ plugin-package-v1 adapter
  └─ miniapp-release-v1 adapter
```

旧 profile 继续验证自己的强约束，新 envelope 只负责统一上层身份、产品投影和能力索引。

### 12.4 统一 Supervisor 外壳，不统一所有 Host 内核

建议抽取统一的 Runtime Supervisor 能力：

- runtime binding；
- generation/fence；
- admission；
- request cancellation；
- watchdog；
- process tree cleanup；
- health；
- startup reconciliation；
- capacity/lease；
- release cutover；
- failure classification。

内部仍保留：

```text
Shared Extension Host
  └─ 多个 headless capability Mount

Dedicated Service Host
  └─ 一个 Extension Service / 一个进程

Surface Host
  └─ iframe + nonce + MessageChannel + session

Build/Test Host
  └─ 一次性构建或验证进程
```

统一的是 admission、fence、observability 和生命周期外壳，不是每一种 Host 的消息集合。

### 12.5 统一 Catalog，但保留角色化 provenance

现有 Catalog 已经是正确的统一方向：

- capability；
- contract digest；
- source identity；
- Plugin Mount 或 MiniApp Active Release provenance；
- consumer；
- availability；
- release admission。

下一步可将 provenance 泛化为：

```text
ExtensionContributionProvenance
  ├─ extension_id
  ├─ installation_id / product_id
  ├─ artifact_or_release_digest
  ├─ runtime_role
  ├─ active_epoch?
  ├─ service_run_key?
  └─ catalog_digest
```

但不能删除 MiniApp 的 epoch 或 Service run key。它们应成为 `runtime_role = service/surface` 时的角色化字段。

### 12.6 “默认可拥有全部能力”需要安全化表达

用户提出“统一都是插件，默认可拥有全部能力”。从产品表达上，可以不再用“这个插件属于哪一类”限制作者；但从运行时安全上，不建议无条件授予全部能力。

当前契约已经采用以下安全原则：

- Manifest 明确声明 contribution；
- Catalog 按 consumer 分开记录 availability；
- Credential 只传 slot 到 credential ID 的引用，不回显 secret；
- Surface、Service、Mount 都绑定 owner、Release、generation；
- 不支持的 role 或 resource fail closed；
- Agent、Gateway、UI 等消费者分别进行 exact lock。

建议把产品语义定义为：

> 插件可以声明和组合平台支持的全部能力类型；每项能力仍需经过 Manifest 校验、消费者准入、用户授权、凭据绑定和运行时安全边界检查。

也就是说，统一概念不等于 ambient privilege。否则一个看似纯 UI 的插件也可能获得 Service、网络、文件和凭据能力，既违背当前设计合同，也会显著扩大攻击面。

## 13. 数据模型统一与迁移建议

### 13.1 不建议直接重命名现有表

当前数据库已经明确把两套数据根分开：

- Plugin：`067_plugin_n1_data_root.sql`；
- MiniApp：`072_miniapp_m1_data_root.sql` 及其后续 077、078、080、081、082、083 等 migration。

`072_miniapp_m1_data_root.sql:1-6` 明确写明新 M1 数据根独立于旧 `miniapps` 表，不读取、不复制、不 alias。

`077_miniapp_build_operation_lineage.sql` 又明确把 MiniApp Build lineage 单独保存，因为其 Build 成功需要同时原子提交自身 Artifact、Release、Ready 和 Operation。

这意味着直接执行以下操作都不安全：

- 把 `miniapp_products` 重命名为 `plugin_mounts`；
- 把 `miniapp_releases` 直接复制成 `plugin_artifacts`；
- 把 `active_release_epoch` 丢弃；
- 把 MiniApp managed Files/DB 合并进 Plugin `dataDir`；
- 把 Plugin Uninstall 语义套在 MiniApp Product 上；
- 把 MiniApp Permanent Delete 语义套在普通 Plugin Mount 上。

### 13.2 建议建立 canonical Extension projection

第一阶段不移动底层数据，而是在应用层建立统一读模型：

```text
Canonical Extension Item
  ├─ extension_id
  ├─ owner_id
  ├─ display metadata
  ├─ source state
  ├─ artifact/release summary
  ├─ roles: capability / surface / service
  ├─ consumers
  ├─ lifecycle projection
  ├─ runtime status
  ├─ data policy
  └─ legacy_origin: plugin | miniapp
```

映射建议：

| 当前对象 | 统一模型中的映射 | 迁移注意事项 |
| --- | --- | --- |
| Plugin Project | Extension Project | 保留原 project/source/lock digest |
| Plugin Artifact | Extension Artifact | 保留 artifact digest 和 profile |
| Plugin Mount | Extension Installation/Binding | 不把 Mount 当作产品 identity |
| MiniApp Product | Extension Product | `miniapp_id` 映射为 canonical extension identity |
| MiniApp Project | Extension Project | 保留 source lineage 和 project identity |
| MiniApp Release Artifact | Extension Release/Artifact | 保留 release_id、release digest、manifest digest |
| Ready Candidate | Extension Ready Candidate | 角色化 test/admission |
| Active Release | Extension Active Deployment | 保留 epoch |
| Previous Release | Extension Previous Deployment | 仍受 rollback 语义约束 |
| Service spec | Extension Service Runtime Binding | 保留 `service_run_key` |
| Surface session | Extension Surface Session | 保留 nonce/session/generation |
| Plugin KV/dataDir | Plugin storage profile | 不强行转换为 managed DB |
| MiniApp KV/Files/Private DB | App storage profile | 保留 migration ledger 和 backup 语义 |
| Plugin Uninstall | Installation detach/retained | 不等同 Product Trash |
| MiniApp Trash/Delete | Product lifecycle/delete intent | 不级联合并为普通 uninstall |

### 13.3 Canonical identity 的选择

推荐新模型使用稳定的 `extension_id`，但迁移期同时保存：

```text
extension_id
legacy_plugin_project_id?
legacy_plugin_mount_id?
legacy_miniapp_id?
```

对 MiniApp，最好让原 `miniapp_id` 成为稳定 extension identity 的可追溯来源，而不是重新生成一个导致所有 Surface、Catalog、backup 和 consumer lock 同时失效的新 ID。

对 Plugin，则需要区分：

- Package identity；
- Project identity；
- Installation/Mount identity；
- Extension product identity。

不能因为统一命名，就把这些原本不同的 identity 压成一个字段。

### 13.4 迁移顺序

建议使用：

1. 新增 canonical projection 表或 read model；
2. 从 Plugin/MiniApp 两边生成 projection；
3. 对比 projection 与原始 aggregate；
4. 只读场景切换到 projection；
5. 写操作仍由原 application service 执行；
6. 增加 shadow write/reconciliation；
7. 逐个迁移写 API；
8. 最后再考虑物理数据合并或删除旧表。

迁移期间必须能验证：

- owner 不变；
- source/artifact/release digest 不变；
- Catalog provenance 不变；
- Active/Previous/Ready 关系不变；
- Surface session 在旧 epoch 上全部失效；
- Service storage 和 migration ledger 不丢失；
- Plugin retained data 不被误删；
- Backup 导入仍创建新 identity；
- 失败可以回滚到原 application service。

## 14. API、路由、CLI 与前端产品迁移

### 14.1 当前入口确实重复

前端路由目前同时存在：

- `/plugins`；
- `/mini-apps`；
- `/mini-apps/new`；
- `/mini-apps/create/:draftId`；
- `/mini-apps/:id`。

证据见 `ui/src/renderer/components/layout/Router.tsx:20、116-118、219、270-273`。

后端也分别注册：

- `/api/plugins`、`/api/plugin-projects`、`/api/plugin-mounts`；
- `/api/miniapps` 及其 source/build/publish/service/surface/backup/delete 路由。

证据见：

- `crates/backend/nomifun-app/src/router/plugin_platform.rs:827-908`；
- `crates/backend/nomifun-app/src/router/miniapp_m1.rs:55-157`；
- `crates/backend/nomifun-app/src/router/routes.rs:852-877、1212-1214、1276`。

CLI 也有独立顶层 `plugin` 和 `miniapp` subcommand，见 `crates/backend/nomifun-app/src/cli.rs:228-232、457-474、782-1023`。

### 14.2 推荐的产品迁移

目标产品形态：

```text
Plugins
  ├─ All
  ├─ Capability
  ├─ UI
  ├─ Service
  └─ Full-stack
```

Library 卡片显示角色和消费者，而不是显示产品类型“Plugin/MiniApp”：

- `Capability`：向系统注入能力；
- `UI`：可打开产品 Surface；
- `Service`：有后台运行能力；
- `Full-stack`：同时拥有 UI 和 Service；
- `Consumers`：Agent、Gateway、UI、Automation 等。

打开行为按角色显示：

- 纯能力插件：Configure、Enable、Test、Inspect；
- UI 插件：Open、Configure、Enable；
- Service 插件：Service status、Retry、Stop、Logs；
- 全栈插件：Open、Service status、Configure、Publish/Rollback。

不要让所有卡片都显示所有按钮，否则只是把两个入口的复杂度搬到了一个页面。

### 14.3 路由/API 兼容策略

建议以 `/api/plugins` 作为最终公共入口，但不要立即删除旧 API：

```text
Canonical:
  /api/plugins
  /api/plugins/{id}
  /api/plugins/{id}/build
  /api/plugins/{id}/test
  /api/plugins/{id}/apply-or-publish
  /api/plugins/{id}/surface/open
  /api/plugins/{id}/service/retry

Compatibility:
  /api/miniapps/*
    → 解析 legacy miniapp_id
    → 转到 Extension application service
```

兼容期需要保留的内容：

- `/mini-apps` 自动跳转到 `/plugins?legacy=miniapp`；
- 外部书签和深链接可继续打开对应 Plugin；
- `miniapp` CLI 作为隐藏 alias；
- API 返回 `Deprecation`/迁移提示；
- 备份和分享格式继续识别旧字段；
- 日志和错误码同时包含新角色和旧兼容名。

### 14.4 Creator 的统一方式

创建流程不再让用户先回答“你要创建 Plugin 还是 MiniApp”，而是让用户描述目标：

```text
“我要一个可以读取发票、提供查询界面、每天同步一次数据的插件”
```

Agent/Creator 根据需求推导：

- 是否需要 Surface；
- 是否需要 Service；
- 需要哪些 Capability；
- 需要什么 storage；
- 面向哪些 consumer。

然后生成一个统一 Project 和 Manifest。高级用户仍可以在 Workshop 中编辑这些角色。

## 15. 分阶段实施路线

### 阶段 0：冻结统一产品定义

目标：先统一语言，不改持久化数据。

工作项：

- 明确 Plugin/Extension 是总概念；
- 定义 Surface、Service、Capability、Full-stack 等角色；
- 明确 MiniApp 只作为兼容标签；
- 明确“可声明全部能力”不等于无条件授予权限；
- 为每个角色写产品、权限和运行时边界。

退出条件：

- 产品、设计、前端、后端、测试使用同一套术语；
- 不再新增以 MiniApp 为中心的独立产品能力。

### 阶段 1：统一导航和产品投影

工作项：

- 新增统一 `Plugins` Library；
- 将 Plugin Library/Project 和 MiniApp Product/Project 映射到统一卡片；
- 角色化显示 UI/Service/Capability；
- `/mini-apps` 改为兼容跳转；
- 侧栏只保留一个入口；
- 保留 MiniApp 专属的 Service/Surface 操作，但放在 Plugin Detail 内。

这一步可以只建立前端和 read-side projection，不需要立即迁移数据库。

退出条件：

- 新用户看不到两个一级产品；
- 原有 MiniApp deep link 仍可用；
- Plugin 和 MiniApp 的 Library 数据都能在统一列表正确显示；
- Enable/Open/Configure/Service status 等行为不串线。

### 阶段 2：抽取 Canonical Extension Domain

工作项：

- 定义 `ExtensionId`、`ExtensionProject`、`ExtensionArtifact`、`ExtensionDeployment`；
- 定义统一 owner/source/build/provenance；
- 建立 Plugin/MiniApp 两个 adapter；
- 把 Catalog、Operation、Test receipt、Import/Export 统一成 envelope；
- 保留 role-specific extension fields。

退出条件：

- 新的只读 API 不再需要调用方判断 `plugin` 或 `miniapp`；
- 原始数据和 projection 有自动一致性检查；
- projection 失败时不影响原产品链。

### 阶段 3：统一 Manifest 与 Build Foundation

工作项：

- 引入 `extension-bundle-v1` 或等价上层 envelope；
- `plugin-package-v1` 和 `miniapp-release-v1` 作为兼容 profile；
- 统一 source snapshot、dependency lock、staging、digest、operation cancel；
- 统一 artifact admission 和 provenance；
- 让新的 Plugin 可以选择 UI、Service 或二者组合。

注意：

- 不要把 UI 文件强行塞进现有 Plugin Package v1；
- 不要让 MiniApp Release 失去 `service/main.mjs`、migrations 或 bridge contract；
- 先兼容，再收敛。

### 阶段 4：统一 Runtime Supervisor 接口

工作项：

- 抽出通用 Runtime Supervisor trait/API；
- 统一 Runtime binding、lease、generation、watchdog、cleanup、health 和 reconciliation；
- 将 Shared Host、Dedicated Service Host、Surface Host 作为不同 role backend；
- 统一故障分类和 observability；
- 将 Runtime switch participant 统一为 Extension runtime participants。

退出条件：

- Runtime Manager 可以列出所有 Extension 角色；
- 切换 Runtime 时能准确 drain/validate/restore 每种 role；
- 不再依赖“所有 Host 都是同一协议”的假设。

### 阶段 5：统一消费者和 Catalog

工作项：

- 统一 capability publication read model；
- 将 UI/Service contribution 也纳入 Extension Catalog；
- 保留 consumer-specific availability；
- 将 Agent、Gateway、UI、Automation 的 exact lock 统一为 Extension contribution lock；
- Surface 和 Service 仍通过各自安全 adapter 执行。

退出条件：

- Catalog 能同时解释 headless、UI、Service 和 Full-stack Extension；
- stale artifact/release/epoch/owner 都能 fail closed；
- 不出现第二个仅针对 MiniApp 或 Plugin 的消费者注册表。

### 阶段 6：数据/API/CLI 迁移

工作项：

- 建立 canonical extension identity；
- MiniApp Product 映射为 Extension Product；
- Plugin Mount 映射为 Installation/Binding；
- 保留 Service storage、Surface session、epoch、migration ledger 等角色记录；
- 新 API 作为 canonical；
- 旧 API 做兼容 alias；
- CLI 的 `plugin` 吸收 `miniapp` 命令，旧命令保留一段兼容期；
- 运行迁移校验、shadow read 和恢复演练。

退出条件：

- 新 API、旧 API 返回同一 authoritative state；
- 导入、备份、删除、回滚和启动恢复有一致结果；
- 迁移失败可以回退，不丢用户数据。

### 阶段 7：退出 MiniApp 用户概念

只有在以下条件全部满足后，才建议物理移除 MiniApp 一级入口和长期兼容路径：

- 所有产品入口已迁移；
- 深链接和旧 CLI 有明确兼容或迁移策略；
- 旧数据已建立可验证映射；
- Backup/Share 版本兼容已覆盖；
- Surface/Service/Catalog/Agent/Gateway/Automation E2E 已覆盖；
- Windows 安装版和目标平台验证通过；
- 运行中 Service、旧 session 和 pending deletion 已可安全 reconcile；
- 已经没有生产可达的 MiniApp-only API 依赖。

## 16. 主要风险与控制措施

### 风险 1：为了统一概念而丢失隔离

表现：

- 让多个 Service 共用普通 Plugin Shared Host；
- 把 Surface iframe 当作普通 Tool；
- 让 Plugin 直接访问 MiniApp Private DB；
- 删除 active epoch 或 Service run key。

控制：

- 保留 role-specific runtime；
- 统一外壳，不统一所有协议；
- 将 epoch、run key、session 作为安全字段；
- 为每种 role 保留独立 conformance suite。

### 风险 2：把“默认全能力”实现成无条件权限

表现：

- 新 Plugin 自动获得网络、文件、凭据、Service、UI 和后台权限；
- consumer availability 用一个全局布尔值代替；
- UI 组件可以自行获得 owner 或 storage。

控制：

- Manifest 显式声明；
- consumer-specific admission；
- Credential 只使用 reference；
- 用户授权和 local-trust gate；
- unsupported role/resource fail closed；
- 保持 Catalog provenance 和 exact lock。

### 风险 3：数据迁移破坏可恢复性

表现：

- MiniApp Backup 导入后 identity 不一致；
- Service storage 和 migration ledger 丢失；
- Plugin Uninstall 的 retained data 被误删；
- 删除操作在中途失败后留下无法恢复的半状态。

控制：

- 不直接改名/合并旧表；
- 先 projection，后迁移；
- deletion intent 和 operation 保留；
- 迁移前后 digest、pointer、owner 对账；
- 先做 shadow read 和恢复演练。

### 风险 4：Release/Package/Project/Mount identity 混淆

表现：

- 用 Package ID 代替安装实例；
- 用 MiniApp Release ID 当作全局产品 ID；
- 一个 Project 误绑定多个 current target；
- 分享导入覆盖现有产品。

控制：

- 明确区分 product、project、artifact、release、installation、runtime；
- 保留原始 ID 作为 lineage；
- Share/Backup 一律按合同创建新 identity；
- 所有切换操作使用 exact CAS。

### 风险 5：统一入口导致 UI 复杂度反而增加

表现：

- 一个页面展示所有 Service、Surface、Capability 的全部控制；
- 用户需要理解 host generation、epoch、run key；
- 纯能力插件和全栈应用的操作混在一起。

控制：

- 角色化卡片和筛选；
- progressive disclosure；
- 默认只展示与当前插件角色相关的动作；
- 将技术状态放入诊断面板；
- Creator 统一，Detail 按角色展开。

### 风险 6：兼容期维护成本上升

表现：

- 新旧 API 双写不一致；
- 旧路由和新路由产生两个 operation；
- CLI alias 与产品状态不一致。

控制：

- 只保留一个 authoritative application service；
- 旧入口只做解析和转发；
- operation identity 不因入口改变；
- 设定明确的兼容期限和移除门槛；
- 增加旧/新入口等价性测试。

### 风险 7：测试矩阵乘法

统一后角色组合会增加测试组合：

```text
Surface × Service × Consumer × Storage × Lifecycle × Runtime
```

控制：

- 用正交 role contract 测试；
- 每个 role 维护最小 conformance suite；
- 共享 digest/CAS/owner/IPC 测试；
- 不把所有组合都实现成独立产品 E2E；
- 先覆盖 headless、UI-only、Service、Full-stack 四个代表单元。

## 17. 建议的验收门槛

### 产品层

- 侧栏和主导航只有一个 Plugin/Extension 入口；
- 用户可以从同一 Creator 创建纯能力、纯 UI、纯 Service 或全栈插件；
- Library 能区分角色，但不再显示 MiniApp 一级产品；
- `/mini-apps` 旧链接可兼容跳转；
- 用户不需要理解两个产品系统。

### 领域模型层

- 一个 canonical Extension identity 可以追踪 Source、Artifact、Release、Installation 和 runtime role；
- Plugin/MiniApp legacy 数据均可投影；
- owner、revision、CAS、digest、provenance 在迁移前后保持一致；
- Ready/Active/Previous 和 Candidate/Test receipt 的关系可验证。

### 运行时层

- headless capability 仍可多 Mount 共享 Host；
- Dedicated Service 仍然一 App 一 Host、可容量控制和失败退避；
- Surface 仍然使用 Host-owned MessageChannel；
- stale Release、epoch、session、generation 全部 fail closed；
- Runtime switch 能覆盖所有 enabled role；
- process tree 和临时 staging 在成功、失败、取消、崩溃后归零。

### 数据层

- UI-only 不获得 Files/Private DB；
- Service 的 Files/Private DB 与 migration ledger 不丢失；
- Plugin retained data 和 MiniApp Trash/Backup 语义不互相污染；
- Permanent Delete 可重试、幂等、可启动恢复；
- Share 与 Whole-App Backup 仍然区分代码交付和业务数据迁移。

### 消费者层

- Agent、Gateway、UI、Automation 等消费者使用统一 Catalog read model；
- consumer availability 仍按消费者分别计算；
- action/resource/contract digest 漂移时拒绝调用；
- MiniApp Service 不通过普通 Plugin Mount 伪装；
- Plugin 不因进入统一目录而自动获得 Agent 或 UI 权限。

## 18. 当前完成度与实施时机判断

根据 `docs/specs/2026-08-28-agent-capability-platform-v2/PHASE-N1-M1-CLOSURE-TODO.zh.md`：

- Plugin N1 Windows Candidate 的主要功能闭环已记录为完成；
- MiniApp M1 Windows Candidate 的主要功能闭环也已记录为完成；
- 两者的 Service、Bridge、Storage、Catalog、Share/Backup、删除和桌面产品验证已经有较完整实现；
- 但最终 Signed RC、跨平台原生 Gate、某些联合 release 验证和旧兼容链物理清理仍需按台账处理。

这对统一工作的含义是：

1. 现在已经足够成熟，可以开始统一产品入口和公共 read model；
2. 现在不适合在仍有 release/兼容验证未完成时做破坏性数据库和协议重写；
3. 应先把统一工作做成独立的 projection/adapter，避免干扰已经闭合的 N1/M1 candidate；
4. 等统一投影稳定后，再安排真正的迁移和旧入口移除。

从 Git 历史看，Plugin 和 MiniApp 是同一轮重构中并行且有意识地闭合的两条主链：

- Plugin：`863e62f08`、`0ee294724`、`d84b18ca9`、`f529b6f92` 等；
- MiniApp：`e24714a7e`、`13f7d906e`、`b7a8bc6c5`、`b95d04554`、`2520904e9`、`86afa7af6`、`708ef83b7` 等。

因此当前重复是“先分别闭合边界，再发现公共产品抽象应上移”的结果，不是简单的重复代码错误。统一应以抽象收敛和迁移为主，而不是否定已经建立的 role-specific 安全边界。

## 19. 最终结论与决策建议

### 问题一：这个思考是否合理？

**合理。**

尤其是以下判断是正确的：

- 用户不应该同时理解 Plugin 和 MiniApp 两套一级产品；
- Plugin 不应被限制为纯无头能力；
- UI、后台 Service、系统能力和全栈应用都可以是同一扩展体系中的不同组合；
- 统一产品形态能够减少入口、文案、工作台和平台概念。

### 问题二：是否符合当前两大能力系统的设计现状？

**产品层面基本符合，运行时层面只部分符合。**

符合之处：

- Plugin 已经产品化；
- 两者共享大量基础设施；
- Catalog 和多消费者模型已经趋于统一；
- MiniApp action 已能进入 Plugin/Nomi 的共同消费链。

不符合之处：

- MiniApp 的 Surface、Service、storage、migration、test、backup、delete 不是 Plugin 当前 `plugin-package-v1` 的简单扩展；
- 两者仍有不同 identity、release/pointer、host topology 和安全协议；
- 当前设计文档仍保留两个产品的显式边界，需要正式调整设计基线。

### 问题三：能否实施？

**能够实施，且建议现在开始，但应分阶段实施。**

实施难度判断：

| 改造项 | 可行性 | 风险 |
| --- | --- | --- |
| 统一产品名称和主入口 | 高 | 低到中 |
| 统一 Library/Workshop 投影 | 高 | 中 |
| 统一 Catalog/consumer read model | 高 | 中 |
| 抽取 Extension domain | 中高 | 中到高 |
| 统一 Manifest envelope | 中高 | 中 |
| 统一 Supervisor 外壳 | 中高 | 中 |
| 统一所有 Runtime protocol | 低且无必要 | 高 |
| 物理合并现有 DB 表 | 可做但不应优先 | 高 |
| 立即删除 MiniApp API/CLI/模块 | 不建议 | 很高 |

### 建议正式采纳的方向

```text
产品：一切皆 Plugin
领域：一套 Extension 内核
能力：统一 Catalog 与 consumer model
运行时：按 role 保留 Shared Host / Dedicated Service Host / Surface Host
迁移：projection → adapter → canonical API → 兼容退出
```

### 暂不建议做的事情

- 直接把 MiniApp 表重命名为 Plugin 表；
- 直接把 MiniApp Release 当作 Plugin Package；
- 删除 `active_release_epoch`、`service_run_key`、Surface session；
- 让普通 Shared Extension Host 承担所有长驻 Service；
- 把 Surface Bridge 改造成普通 capability invoke；
- 把 MiniApp managed storage 和 Plugin raw `dataDir` 混成一个默认存储；
- 在迁移前删除 `/api/miniapps`、`/mini-apps` 和 `miniapp` CLI；
- 把“默认可拥有全部能力”实现为无条件权限。

最终推荐决策：

> **采纳“Plugin 是唯一对外产品概念”的方向；将 MiniApp 收敛为 Plugin 的 UI/Service/Full-stack 角色组合；统一产品入口、领域内核、Catalog 和 Supervisor 外壳；保留角色化运行时、存储和生命周期协议，并通过渐进迁移最终退出 MiniApp 用户概念。**

## 附录 A：关键代码证据索引

| 主题 | 关键文件 | 证据 |
| --- | --- | --- |
| 原始双产品设计 | `docs/specs/2026-08-28-agent-capability-platform-v2/06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md` | `298-307`：两个并列产品；`430-449`：M1 能力；`493-499`：共享基础；`734-819`：Release、Service、Bridge、capacity |
| Plugin 产品工作台 | `ui/src/renderer/pages/plugins/PluginWorkbenchPage.tsx` | `72-109`：工作区与产品面；`625-889`：产品打开、创建、构建、测试、应用；`941-1011`：配置、导入、测试、分享 |
| Plugin 类型与消费者 | `ui/src/common/types/pluginPlatform.ts` | `23-47`：生命周期与 consumer；`150-216`：Test/Impact；`273-456`：Import/Build/Test/Apply/Configure/Uninstall |
| Plugin Host 方法集 | `crates/backend/nomifun-agent-contracts/src/plugin_n1.rs` | `74-130`：Mount/Invoke/Context/Resource/State/Cancel/Shutdown；`787-906`：请求和方法映射 |
| Plugin Package 限制 | `crates/backend/nomifun-agent-contracts/src/plugin_n1.rs` | `54-57`：两个 build profile；`1109-1190`：Plugin Package 校验；`1385-1425`：Artifact 文件和 digest |
| Plugin Service 限制 | `crates/backend/nomifun-js-kernel-adapter/src/lib.rs` | `45-51`、`98-120`：拒绝 Service/Role Provider，要求 `main.mjs` |
| 通用 Package/Contribution | `crates/backend/nomifun-agent-contracts/src/package.rs` | `378-430`：PackageManifest、CapabilityKind；`451-452`：消费者；`946-952`：registrar operation |
| 共享 Catalog | `crates/backend/nomifun-agent-contracts/src/catalog.rs` | `34-80`：Plugin/MiniApp provenance；`134-191`：release admission；`225-241`：Catalog entry；`294-370`：MiniApp publication |
| Plugin Shared Host | `crates/backend/nomifun-js-host/src/supervisor.rs` | `378-395`：generation；`484-585`：load/invoke；`763-920`：stop/ensure；`1458-1605`：watchdog/fence |
| Plugin 数据根 | `crates/backend/nomifun-db/migrations/067_plugin_n1_data_root.sql` | `8-51`、`199-316`、`431-500`：Artifact/Project/Candidate/Mount/Credential/KV；`358-427`：retained/delete/dataDir 不变量 |
| MiniApp Product 模型 | `crates/backend/nomifun-miniapp-platform/src/model.rs` | `21-24`：UiOnly/Service；`134-192`：Release/Product/DataRoot；`250-317`：结构校验；`487-551`：storage/release 约束 |
| MiniApp Manifest | `crates/backend/nomifun-agent-contracts/src/miniapp_m1.rs` | `412-560`：UI/Service/credentials/bridge/migrations；`1543-1852`：storage、runtime fingerprint、Service spec |
| MiniApp Release cutover | `crates/backend/nomifun-miniapp-platform/src/runtime.rs` | `19-110`：Publish/Rollback/migration；`115-150`：lifecycle；`314-350`：exact Service spec 和 UI-only 限制 |
| Dedicated Service Host | `crates/backend/nomifun-miniapp-platform/src/service_host.rs` | `18`：默认 capacity；`38-50`：generation fence；`227-315`：容量；`343-439`：启动/失败/backoff；`646-716`：idle/reconcile |
| Service process protocol | `crates/backend/nomifun-miniapp-platform/src/service_process.rs` | `254-358`：start/invoke/cancel；`912-950`：fence/Hello；`1450-1465`：storage IPC；`1909-1965`：frame/process 边界 |
| Surface Bridge | `crates/backend/nomifun-miniapp-platform/src/bridge.rs` | `17-20`：Host-owned；`42-76`：binding；`130-232`：session/port；`249-318`：epoch/bridge call；`355-376`：cancel/close |
| Surface 前端 | `ui/src/renderer/pages/miniApps/MiniAppSurfacePanel.tsx` | `45-50`：协议事件；`166-177`：descriptor key；`202-299`：MessageChannel；`310-427`：nonce/handshake；`520-528`：sandbox iframe |
| MiniApp 独立数据根 | `crates/backend/nomifun-db/migrations/072_miniapp_m1_data_root.sql` | `1-6`：独立于旧表；`8-115`：Product/pointer；`117-336`：Project/Artifact/Release；`338-362`：Credential |
| MiniApp Build lineage | `crates/backend/nomifun-db/migrations/077_miniapp_build_operation_lineage.sql` | `1-61`：独立 Build operation lineage |
| Surface session | `crates/backend/nomifun-db/migrations/080_miniapp_surface_sessions.sql` | `8-66`：session、release digest、epoch、generation |
| Delete intent | `crates/backend/nomifun-db/migrations/082_miniapp_deletion_intents.sql` | `1-43`：独立 durable deleting intent |
| Service Test receipt | `crates/backend/nomifun-db/migrations/083_miniapp_service_test_receipts.sql` | `1-102`：Release/run key/runtime/storage receipt |
| 应用组合根 | `crates/backend/nomifun-app/src/services.rs`、`router/state.rs` | `services.rs:1978-1981`：MiniApp facade；`3491-3516`：独立 M1 stores；`state.rs:881-1007`：Catalog 与共同 Tool session |
| 路由分离 | `crates/backend/nomifun-app/src/router/routes.rs`、`plugin_platform.rs`、`miniapp_m1.rs` | Plugin/MiniApp 两组 route 和独立 owner gate |
| 前端入口分离 | `ui/src/renderer/components/layout/Router.tsx` | `219`：`/plugins`；`270-273`：`/mini-apps` 系列 |
| CLI 分离 | `crates/backend/nomifun-app/src/cli.rs` | `228-232`：`miniapp` 顶层命令；`457-474`：MiniApp list/show；`989-1023`：两套命令树 |

## 附录 B：本报告的边界

- 本报告是基于当前工作树的静态代码和设计审查，不替代最终安装版、跨平台和真实用户验收。
- 本次仅新增文档，没有修改业务代码。
- 本次没有运行完整 workspace build/test；文档新增采用 `git diff --check` 和变更范围复核即可。
- 报告建议的 `Extension`、`Surface`、`Service` 等名称是目标架构建议，不代表当前代码中已经存在同名最终类型。
