# Agent Capability Platform v2 一期精简闭合 TODO

> 盘点日期：2026-09-09
>
> 基线分支：`rf/agent-capability-platform-v2`
>
> 权威来源：`05-system-capability-replacement-foundation.zh.md`
>
> 当前结论：S0-S2 的止损、P0 与基础收缩，以及既有 S3 基础切片仍按历史定向
> evidence 保留。2026-09-06 两轮人工走查先后发现首页缺少真实 AgentPreset
> 选择器，以及 Agent 工作台把具体资源错误冻结进 Preset、缺少删除入口和能力
> 明细。旧 AP-7 evidence 已再次撤销；设计纠正随后通过真实 Tauri Desktop 产品走查，
> 并以实现提交 `ef5f5380915e0d5c06004f03b66cbd1302b3fe03` 重新签署 AP-7。
> AP-0～AP-7 与 `SL-S4-02` 均已关闭。
> 产品执行内核仍是 NomiCore/Nomi engine，Web、Desktop 和 `nomicore` 默认入口均使用
> `NomiCoreApplication`。
> 当前实现让首页 Guid pill bar 显示 Agent 工作台中可执行的用户
> AgentPreset，`+` 打开 `/agent`，工作台“使用 Agent”会预选对应 Preset；普通 Guid
> 不覆盖 Preset 的模型、能力模式或 Skills，但会按该 Preset 的能力声明提供会话级
> Workspace、Knowledge 等资源选择。Preset 会话进入后继续锁定 Snapshot 模型，
> 普通 Nomi 则保留模型选择器。
> 标准 Session 创建请求只提交 `preset_id` 与可选 `title`，服务端按 owner 解析当前稳定
> Revision 和持久化 Snapshot，先构造资源中立 Binding；Snapshot 冻结
> `required_resource_kinds`，具体资源随后绑定到消费目标。
> 执行引擎只属于基础设施，
> 不再伪装成产品 Agent。
> Fresh-v4 的开发期漂移也已修正为“只 seed 官方模板、Revision 使用 `payload_json`
> 与 ContributionLock、没有旧 preset 子表/投影”；workspace、UI、DB、Fresh/Nomi-core
> E2E、全量 app lib、generated contract、真实 Tauri Desktop 和真实 StepFun smoke
> 均已通过。Windows x64 Nomi-core 候选随后在提交
> `0bac72da4ebb62f6a0f183a1285065c88aa684a4` 上完成 package/install/fresh/launch、
> 真实 StepFun、Browser/Computer、Remote、进程树清理和卸载验证，`SL-S5-01` 已关闭。
> 用户已明确授权继续实施 06；Plugin/MiniApp 使用独立的
> `PHASE-N1-M1-CLOSURE-TODO.zh.md` 跟踪，不回填到本期 S0-S5 数量中。

本文是 05 发布后的唯一一期执行台账。旧版 84 个 `INF/W/LEG/SCN/TST/REL`
ID 从现在起只作为历史审计索引，不再是一期必须逐项关闭的阻断清单，也不得继续用
“81 个 action-bearing Capability 是否全部有 owner”、旧 residual 数量或五平台笛卡尔积
衡量一期完成度。

一期执行台账追踪 05 经 2026-09-03 修订后的 S0-S5，并在其上增加 AP-0～AP-7 前置
收口层：先停止错误扩张和审计普通 revert，
再关闭三个 P0、单 Compiler、小 Snapshot 和三类 Effect；Codex upstream spike 只作为
历史研究保留。当前继续完成 Browser/Computer Role seam、Nomi-core 真实 owner、automation、
Remote、四条 UI 用户流程以及 Windows、macOS arm64、Linux Desktop 的 Nomi-core RC。
C9/Nomi 删除和 Nomi-free RC 已延后，不新增当前阶段阻断。

## 状态与执行规则

| 状态 | 含义 |
| --- | --- |
| `open` | 当前 owner 可以直接实施，不需要等待其他 TODO |
| `blocked` | 依赖项或当前必需的 live binary/credential 尚未具备；不得通过兼容层、mock 或 synthetic PASS 绕过 |
| `external` | 只有对应真实原生平台或签名环境才能产生发布证据；不是跨机开发任务 |
| `pending-validation` | 实施内容已形成，但尚待审查、提交或指定验证 |
| `deferred` | 已明确移出当前阶段；保留设计依据，但不领取、不重试、不计入当前交付阻断 |
| `closed` | 完成定义已满足，并有提交或可复查的最小 evidence |

执行约束：

1. 05 与本文冲突时以 05 为准；经修订的 01-04 与 `DECISIONS` 保留设计依据，但不记录实时
   状态。旧 `IMPLEMENTATION-STATUS`、旧 GLOBAL TODO、旧 Prompt 和旧 handoff 仅作 Git
   历史审计，不是当前执行材料。用户于 2026-09-09 明确要求阶段性交接时，当前唯一的
   启动入口是 `CROSS-MACHINE-WORK-START-PROMPT-2026-09-09.zh.md`；它不替代本文、
   PHASE 台账或 05/06 设计合同。
2. 不使用 reset、force-push 或历史重写；revert 必须使用普通提交，并先检查真实消费者。
3. 每个任务只实现一个实际闭环；需要第二份事实、新 coordinator、新全局 digest 或新状态机
   时先停止并重新核对 05。
4. 非首批 Capability 可以保持明确 unavailable，但不得返回 metadata-only success，也不得
   阻塞核心 Stable。
5. API key、token、私钥、主机地址和签名 secret 不进入源码、文档、fixture、日志、命令行
   参数或报告。
6. 测试遇到环境或 harness 障碍时记录首个完整失败、停止盲目重试，并提供人工替代步骤。
7. 二期 `06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md` 不属于本台账；
   AP-0～AP-7 和 Windows C8 已完成，用户已单独授权其实施，实时状态由独立 N1/M1
   台账维护。
8. Codex app-server 只作为未来 host boundary 研究；其 fixture、adapter、Broker smoke
   和 live binary 缺口不阻塞当前 Nomi-core 工作。

## 2026-09-06 AgentPreset AP-0～AP-7 收口检查点

> 本节是当前实现提交的增量台账，优先级高于下方旧 S0-S5 快照中对 Agent Settings
> 的局部“已关闭”描述。它不修改 05 §15 的合同，只记录当前代码能证明到哪一步。
>
> `open` 表示仍可在本机实施；`pending-validation` 表示已有实现切片但尚缺合流/
> 行为证据；`blocked` 表示门禁前置条件未满足。任何 AP 项都没有因为文档更新而自动
> 变成 `closed`。

<!-- AP_STATUS AP-0: closed -->
<!-- AP_STATUS AP-1: closed -->
<!-- AP_STATUS AP-2: closed -->
<!-- AP_STATUS AP-3: closed -->
<!-- AP_STATUS AP-4: closed -->
<!-- AP_STATUS AP-5: closed -->
<!-- AP_STATUS AP-6: closed -->
<!-- AP_STATUS AP-7: closed -->

| AP | 状态 | 当前可证明的事实 | 尚未满足的条件 |
| --- | --- | --- | --- |
| AP-0 | `closed` | Agent 工作台唯一入口、owner 矩阵、平台供给层边界和 06 依赖已写入实现与架构文档；`/agent` 是公共入口。 | 无；06 已由用户单独授权并转入独立台账。 |
| AP-1 | `closed` | 共享 Catalog materializer/resolver 已区分 host surface 与 consumer；Agent catalog 过滤 `consumer=agent`；Gateway 与 Agent 对 `knowledge.search` 共享同一 resolver；Knowledge-only `browser.render_content` 被 Agent 过滤。 | 无。 |
| AP-2 | `closed` | Revision payload、ContributionLock、digest、Snapshot、Preview/Save/Session boundary 已统一；`resource_bindings/resource_binding_refs` 已从 Preset Revision/Snapshot 删除，模板只声明 `required_resource_kinds`；未知字段和客户端内部 lock 均 fail-closed。 | 无。 |
| AP-3 | `closed` | Agent 工作台显示每项能力的名称、说明、来源、可用性、action/context 数和资源种类，并提供关闭/启动即用/可按需申请三态；用户 Agent 有删除入口；首页 pill selector、`+` 与工作台预选已接通；Guid 只在消费目标选择 Workspace/Knowledge。真实 Tauri Desktop 已验证创建、三态保存、删除、预选、默认 Nomi 和 Preset 会话。 | 无；手机模式不属于 `nomifun-desktop` 本期服务与验收范围。 |
| AP-4 | `closed` | 标准 `/api/agent-sessions` 请求只含 `preset_id` 与可选 `title`；服务端按 owner 从 `current_stable_revision` 和持久化 Revision/Snapshot 构造初始资源中立 Binding。Snapshot 冻结模型与 `required_resource_kinds`，Preset 会话拒绝公开 PATCH 改写顶层模型；目标资源不进入 Preset/Snapshot；旧 `agent_binding` 请求字段 fail-closed；删除 Preset 后已有 Session 继续读取冻结能力和历史，新 Session 被拒绝。 | 无；control-plane、Conversation、AgentPlatform E2E 与 Nomi-core route 已覆盖。 |
| AP-5 | `closed` | ContributionLock impact diff 已覆盖 Compatible/Breaking、Disabled/Unavailable/Uninstalled、MiniApp release change、Retry/Switch/Restore/Fork，并提供 owner-scoped impact API。 | 无。 |
| AP-6 | `closed` | 旧 `nomifun-preset`、旧 DTO、Extension preset contribution、旧 DB repository/表和旧 UI authoring surface 保持删除；Fresh-v4 bootstrap 只 seed `agent_preset_templates`，Revision 使用 `payload_json` 与 canonical ContributionLock；065 增加用户 Preset retirement tombstone；066 退役旧资源绑定 Preset 并清理活动 Binding，保留 Revision/Snapshot/Session 历史；旧 capability/skill/resource projection 表不再写入。 | 无；generated contract、DB migration head 66 和 workspace 编译已通过。 |
| AP-7 | `closed` | 旧签署均保留为 revoked 历史；实现提交 `ef5f5380915e0d5c06004f03b66cbd1302b3fe03` 覆盖能力三态/明细、用户删除、资源中立 Preset/Snapshot、冻结模型与资源种类、target-scoped resource binding、Cron host Snapshot、Nomi on-demand ToolSearch、deny-all 和删除后历史 Session 保留；Desktop、broad checks 与真实 StepFun smoke 均通过，signed evidence 已形成。 | 无；06 已满足 AP 前置并由用户显式启动。 |

### AP-6 clean-cut 事实、Fresh-v4 漂移纠正与允许的历史残留

- `nomifun-preset` 已不再是 workspace member 或 Cargo dependency。Gate 只检查真正的
  dependency key、`Cargo.lock` package entry 和 Rust import；删除期间根
  `Cargo.toml` 的 `exclude = ["crates/backend/nomifun-preset"]` 可以作为空目录 tombstone
  记录，不能被当成 production dependency。
- `crates/backend/nomifun-db/migrations/001_v3_baseline.sql` 中的
  `preset_snapshot` 是历史 baseline bytes；它不是当前 schema alias。`061_agent_snapshot_naming.sql`
  对 `conversations`、`agent_execution_participants`、
  `agent_execution_template_participants` 和 `cron_jobs` 做物理列重命名，不做双读写或
  compatibility view。
- `062_agent_preset_contribution_locks.sql` 持久化 Revision ContributionLock；
  `064_agent_preset_revision_payload.sql` 把 Nomi 数据线的
  `editor_document_json` 物理重命名为 `payload_json`，不保留 alias 或双读写。
  Revision digest 必须覆盖 payload 与 lock 集合。`preset_id`/`preset_revision`
  只作为 AgentPreset provenance，不能与已删除的旧 preset 快照模型混为一谈。
- `065_agent_preset_retirement.sql` 为用户 AgentPreset 增加 owner-scoped retirement
  tombstone；`066_retire_resource_bound_presets.sql` 扫描旧 Revision/Snapshot 的具体
  资源字段，删除活动 Agent/Remote Binding 并退役对应 Preset，但不删除不可变
  Revision、Snapshot 或历史 Session。SQLx 通过 `nomifun-db/build.rs` 跟踪 migration
  目录，当前 migration head 为 66。
- canonical `ResolvedSnapshotContent` 直接冻结 `required_resource_kinds` 并参与
  Snapshot digest。Nomi Conversation projection 只读取这一冻结集合决定资源入口，
  不再按 Capability ID 从当前 Catalog 合并不同版本。
- Fresh-v4 canonical baseline 的 `agent_preset_templates` 只允许
  `source_kind=official`，没有 Package source foreign key；bootstrap 只写模板 seed，
  不自动创建系统 AgentPreset/Revision。`agent_preset_revisions` 使用 `payload_json`，
  ContributionLock 使用 canonical lock 表；旧 capability/skill/resource projection
  表不属于当前 schema，也不得由 bootstrap 继续访问。
- 本轮 Fresh AgentPlatform E2E 在进入路由前暴露了旧 bootstrap 仍读取
  `source_package_id` 并写旧投影表的漂移。该生产代码已删除，Fresh host restart、
  Fresh-v4/DB/E2E 和 generated contract 已重跑通过。
- 删除合同 JSON、历史 migration 和回归测试中出现旧名称时，Gate 将其单独标为
  historical/test；这不等于允许活动路由、应用服务、DTO、UI consumer 或 generated
  inventory 保留旧主链。

### AP-7 纠偏验证结果

| 命令 | 当前结果 | 说明 |
| --- | --- | --- |
| 旧提交上的 `bun run gate:agent-v2 -- ap-7`、workspace/UI/contract checks | `superseded` | 这些结果发生在真实 AgentPreset selector 和资源边界纠正前，只保留历史审计，不能签署当前工作树。 |
| `bun test --cwd ui` | `pass` | `3211 passed / 0 failed`；覆盖 Guid AgentPreset 选择、默认 Nomi 模型选择、Preset 会话模型锁定、删除、能力三态、资源中立与桌面会话级 Workspace/Knowledge。 |
| `bun run build:ui`；i18n/icons/dead-css/vocabulary | `pass` | 当前工作树 production build 与专项门禁通过；全量 typecheck 仍有仓库既有 React 19/Arco 基线。 |
| `cargo test -p nomifun-agent-control-plane --lib`；`cargo test -p nomifun-agent-kernel --lib`；`cargo test -p nomifun-api-types --lib` | `pass` | 分别 `16/18/530 passed`；覆盖 owner、stable Revision/Snapshot、资源种类 digest、删除 retirement、旧 Binding 输入拒绝和模板 required resource kinds 合同。 |
| `cargo test -p nomifun-v4-root -- --test-threads=1`；`cargo test -p nomifun-agent-platform --lib`；`cargo test -p nomifun-agent-platform --test chat_minimal` | `pass` | Fresh-v4 root `11 passed`、AgentPlatform lib `16 passed`；资源中立 Snapshot、target resource binding 和 chat.minimal 零能力 profile 通过。 |
| `cargo test -p nomifun-app --test agent_platform_e2e`；`cargo test -p nomifun-app --test nomi_core_route_gap -- --test-threads=1` | `pass` | Fresh E2E 与 Nomi-core route gap 通过，包含无 owner capability unavailable、on-demand、删除后历史 Session 和新建拒绝。 |
| `cargo test -p nomifun-conversation --test conversation_crud -- --test-threads=1` | `pass` | `34 passed`；普通 Nomi 仍可换模型，Preset 会话顶层模型 PATCH fail-closed，协作与目标 Workspace 更新仍可用。 |
| `cargo test -p nomifun-app --lib -- --test-threads=1` | `pass` | `405 passed / 1 credential-only ignored / 0 failed`。 |
| `cargo test -p nomifun-app --test cron_e2e -- --test-threads=1` | `pass` | `34 passed`；Host 在 Cron 持久化前把稳定 AgentPreset Revision/Snapshot 冻结到 `agent_config.agent_snapshot`。 |
| DB 064/065/066 migration、ID schema、migration lineage | `pass` | 064 物理 rename、065 retirement tombstone、066 资源边界退役、历史保留和 SQLx migration head 66 通过。 |
| `cargo check --workspace`；generated contract check | `pass` | workspace 编译和 canonical generated artifact check 通过。 |
| 真实 Tauri Desktop 产品走查 | `pass` | 桌面窗口验证默认 Nomi 模型选择、AgentPreset pill/工作台预选、能力三态与明细、删除、Preset 会话无模型选择器、桌面 Header 的目标级 Knowledge/Workspace；无横向溢出或控件重叠。手机模式不在本期范围。 |
| Credential Manager runner 的真实 StepFun smoke | `pass` | 2026-09-06 按当前资源中立 Session、冻结模型/资源种类与目标绑定语义重跑，输出 `live_smoke_status=pass code=OK status=200`。 |
| `node --check scripts/gate-agent-v2.mjs`；Gate self-test；`bun run gate:agent-v2 -- ap-7` | `pass` | 签署提交上验证 evidence、实现提交祖先关系、clean worktree、migration 061/062/064/065/066、UI/模型冻结和 residual，admission=`admitted`。 |

AP-7 的 admission 必须同时满足：AP-0～AP-6 全部 `closed`、旧活动主链 residual 为 0、
generated inventory 已同步、真实 Agent 与非 Agent consumer 证据可复查、ContributionLock/
impact/no-fallback 行为测试通过，并在干净提交上形成签署记录。在这些条件满足前，06 仅
允许设计审阅和文档修订。

## 2026-09-06 Windows C8 候选闭合

- 候选源码：`0bac72da4ebb62f6a0f183a1285065c88aa684a4`。
- Gate：`bun run gate:agent-v2 -- c8-win-pre`，最终 `pass`，29 个记录检查、10 条
  产品流程全部通过。
- Host SHA-256：
  `555a45607272939d9cc89c9f2ef10f850f570cac81e32979afd3fce04770a80c`。
- NSIS Package SHA-256：
  `d0f22840934ddbc69dcd365e9e14899ea17f8c739ed251e572c059f652ec64cf`。
- release lock SHA-256：
  `54e39e87404d43946de6a9c143816fa53367303a4b3928ab6b5966672f6ab266`；
  `sidecars={}`，当前 Nomi-core 候选不要求 Codex Sidecar。
- 安装版 smoke：静默安装返回 0；安装后的 `nomifun-desktop.exe` 写出真实
  `port.json`，`/health` 返回 200/`ok`，WebView2 CDP 页面为
  `http://tauri.localhost/#/guid`、标题 `NomiFun`；进程树退出、静默卸载和
  主程序/注册表清理均通过。
- 真实 StepFun Coding Plan `step-3.7-flash` 再次输出
  `live_smoke_status=pass code=OK status=200`；凭据只从 Windows Credential Manager
  注入测试进程，不进入 Git 或 evidence。
- Fresh-v4 Remote `observe` 已收敛为历史只读查询，不再因 Runtime admission、
  Sidecar 不可用或 RemoteBinding 删除返回时序相关的 503。
- 手机模式不属于 `nomifun-desktop` 服务与验收范围，C8 未运行手机视口。

## 历史进度保留

以下是真实已完成或可复用的功能，不因止损而删除，但也不自动关闭后续集成任务：

| Commit | 保留内容 | 后续处置 |
| --- | --- | --- |
| `099893cc`、`56e70fd1d` | Fresh-v4 storage generation 与前端 bootstrap 启动修复 | 保留，继续作为 `bun run dev` 基线 |
| `745fabfa` | binding-backed `knowledge.search/read` owner | 保留真实 owner |
| `280841b3` | KnowledgeBase picker | 从 AgentPreset 编辑器移出，保留为会话/伙伴等消费目标的资源选择交互 |
| `5d691824` | anchored Knowledge 文件访问 | 保留基本 containment；不继续扩大极端本机攻击证明 |
| `3f835174`、`c6503a23` | canonical AgentSession command/query ServiceKey 及 core service package host 测试适配 | 保留单一 Session authority |
| `8aade375` | 真实 local/file `vcs.push` owner | 保留 owner；按三类 Effect forward 简化 |
| `b58a0f92` | fork cursor 修复 | 保留产品语义 |
| `dd07b937` | Remote public route 改用 Session command/query ports | 保留；auth fence 已由 `SL-S2-03` 关闭，当前 Nomi-core Remote 产品链仍待 `SL-S3-11` |
| `efbcb598`、`23e039ff`、`1a547f3a`、`f46cc017` | Knowledge 的 Windows/macOS 工程验证 | 保留为工程记录，不冒充最终候选 native PASS |

已按止损结论处理的历史实现：

- `d1acccf6` Wave 3 批量 typed contract：已由 `2ad8ca12` 普通 revert。
- `765d1953` Wave 4 通用 Effect/receipt contract：已由 `8f4ba1d9` 普通 revert。
- 旧 C8/C10 cohort、handoff、fixture digest、五格 evidence 和 residual 分类实现：
  只保留修复 P0、三平台 RC 和发布追溯真正需要的最小部分；旧交接材料不再作为当前任务
  入口。

## 已完成基础切片与定向 evidence

下列项目已经在当前分支中实现，并有可复查的定向 evidence。这里的 `closed` 只表示
对应基础切片满足完成定义，不表示 Windows 候选、C8 或原生平台发布
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
| `SL-S3-07` Nomi-core 真实 owner | `powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\validation\run-nomi-core-live-provider-from-windows-credential-manager.ps1`；`cargo test --locked -p nomifun-app --test nomi_core_route_gap -- --test-threads=1`；`cargo test --locked -p nomi-tools vcs::tests --lib`；`cargo test --locked -p nomifun-knowledge --lib` | 真实 StepFun Plan provider、Nomi-core Session、Chat/Coding、Workspace/File、Process、VCS、Knowledge search/read、Cron 同 Session run/replay、Remote open/turn/observe/cancel、凭据持久化审计和关闭清理均通过；输出 `live_smoke_status=pass code=OK status=200`。该 smoke 不代替其他合同，但 `SL-S3-10`、`SL-S3-11`、`SL-S4-02` 后续均已由独立证据关闭 |
| `SL-S3-11` Nomi-core Remote REST/MCP | `powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\validation\run-nomi-core-live-provider-from-windows-credential-manager.ps1`；`cargo test --locked -p nomifun-app --test nomi_core_route_gap -- --test-threads=1`；`cargo test --locked -p nomifun-public --lib` | 同一真实 StepFun smoke 已通过 REST 与 Streamable HTTP MCP 的 `initialize/tools.list/open/turn/observe/cancel`；route-gap 覆盖 installation Bearer、owner JWT/local-trust、旧 selector query、revoke、delete 后不可复活；公共 transport 保留四工具精确集合和 transport-session admission |

## 汇总

| 阶段 | closed | open | blocked | external | pending-validation | deferred | 合计 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| S0 止损发布 | 2 | 0 | 0 | 0 | 0 | 0 | 2 |
| S1 Revert/keep 审计 | 3 | 0 | 0 | 0 | 0 | 0 | 3 |
| S2 P0 与基础收缩 | 10 | 0 | 0 | 0 | 0 | 0 | 10 |
| S3 Role seam 与 Nomi-core owner | 11 | 0 | 0 | 0 | 0 | 1 | 12 |
| S4 产品 UI | 2 | 0 | 0 | 0 | 0 | 0 | 2 |
| S5 三平台与 Nomi-core RC | 1 | 0 | 1 | 2 | 0 | 1 | 5 |
| **总计** | **29** | **0** | **1** | **2** | **0** | **2** | **34** |

旧台账 84 项现已收敛为 34 项。任务数量不是质量指标；只有完成定义和最小验证满足后
才能修改状态。

上表只统计 S0-S5。AP-0～AP-7 是额外的 AgentPreset 门禁层；截至
2026-09-06，AP-0～AP-7 已有本机证据并关闭；用户随后已明确授权 06 代码实施。
N1/M1 的状态不计入上表，统一由独立台账维护。

## 当前剩余 TODO 快照

S0-S5 当前还剩 5 项未关闭，其中 2 项已明确延后。Windows 候选已经关闭；用户要求
先完成 06 的 Windows Plugin/MiniApp 开发，再统一交接外部原生环境：

| 分类 | 数量 | TODO |
| --- | ---: | --- |
| 主机当前实施 | 0 | 无；本期 Windows 候选已关闭，主机转入独立 N1/M1 台账 |
| 依赖阻塞 | 1 | `SL-S5-05` 等待同一候选的三平台结果 |
| 外部原生环境 | 2 | `SL-S5-02` macOS arm64、`SL-S5-03` Linux Desktop x64 |
| 后续阶段 | 2 | `SL-S3-12` Codex app-server、`SL-S5-04` C9/Nomi 删除 |

主机关键路径已完成 `SL-S3-01 -> SL-S3-02 -> SL-S3-03 -> SL-S3-07 -> SL-S3-10`，Browser/Computer
first-party dogfood、具体实现旁路清理和 `SL-S5-01` 也已完成。当前主线转为 06 的
Windows Plugin/MiniApp；Codex Sidecar 和 C9 不在当前关键路径。
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
| `SL-S2-08` | closed | 主机 | 缩小 Snapshot、CapabilitySelection 和 Fresh-v4 投影 | `SL-S2-07` | Snapshot 只锁实际 Capability/Provider/Tool/Model/runtime 闭包和 required resource kinds；具体 target resource 不进入 Snapshot；删除未执行 selection 字段和只写不读的重复投影；fresh-v4 fixture 可双启动 | `cargo test --locked -p nomifun-v4-root -- --test-threads=1`; `cargo test --locked -p nomifun-agent-kernel --lib` | 无 |
| `SL-S2-09` | closed | 主机 | 简化 PluginRegistration | `SL-S2-07` | Manifest 是声明事实源；registration metadata 从真实 handler/service exports 派生；保留 namespace、schema、typed dependency、duplicate/cycle 和 cleanup | `cargo test --locked -p nomifun-agent-kernel materialize --lib` | 无 |
| `SL-S2-10` | closed | 主机 | 完成 Codex official app-server upstream 协议研究 | 无 | pinned source 已确认 initialize/thread/turn/interrupt/event、Host-managed Tool 和关闭语义；不预设历史自定义 RPC；结果只作为未来 host boundary 输入，不代替 source build、真实 binary、模型、工具或产品 Coding 验证 | `a7ac1d124`; `bun scripts/validation/codex-app-server-spike.mjs --self-test`; `bun test scripts/validation/codex-app-server-spike.test.mjs` | 本项是研究关闭，不表示 Codex-native 完成；`SL-S3-12` 已延后 |

## S3：Role seam 与核心 owner

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S3-01` | closed | 主机 | 冻结 Browser/Computer versioned Role 合同 | `SL-S2-09` | `ExecutionRoleId`、Role Contract、source-neutral Provider contribution、required/optional member、typed Context/Resource exports 只有一套 canonical Rust/schema 定义 | `cargo test --locked -p nomifun-agent-contracts role --lib`; `cargo run --locked -p nomifun-agent-contracts --bin agent-v2-contract -- check`; `cargo test --locked -p nomifun-agent-kernel --lib` | 无 |
| `SL-S3-02` | closed | 主机 | 实现 installation binding、Revision override、Resolver 和 Snapshot exact lock | `SL-S2-07`、`SL-S2-08`、`SL-S3-01` | override 优先、缺省继承 installation default；精确 Provider/contract/contribution 和资源种类约束进入 Snapshot digest；target resource 仅在 operation/Session admission 注入；缺失明确失败且不 fallback | `cargo test --locked -p nomifun-agent-control-plane --lib`; `cargo test --locked -p nomifun-agent-kernel --lib`（含 alternate/provider/registry/resource drift） | 无 |
| `SL-S3-03` | closed | 主机 | 实现单一 RoleDispatcher 与 Tool/Context/Resource runtime seam | `SL-S3-02` | Kernel 第一次路由直接选 frozen Provider Mount；Agent 与 non-Agent Tool/Context/Resource 共用 exact resolver；使用 Provider config/state/service/resource；不 façade 二次调用、不重选、不 retry/fallback | `cargo test --locked -p nomifun-agent-kernel --lib`（18/18）; `cargo check --locked -p nomifun-app --features browser-use,computer-use` | 无 |
| `SL-S3-04` | closed | 主机 | 第一方 Browser dogfood 同一 Role 主链 | `SL-S3-03` | observe/navigate/act 和 hidden `browser.render_content` 经同一 Provider lock；保留 owner/lane/close/process cleanup；Provider 平台约束不写死在 façade | `cargo test --locked -p nomifun-app --features browser-use --lib browser_role_owner_runs_the_canonical_observe_navigate_act_render_chain -- --ignored --test-threads=1`；Wave2 owner/lifecycle tests；alternate Provider parity test | 无；本机 data URL 作为可访问测试页 |
| `SL-S3-05` | closed | 主机 | 第一方 Computer/A11y dogfood 同一 Role 主链 | `SL-S3-03` | observe/input 基线和可选 launch/a11y 经 exact Provider；按 target resource 串行；observation generation 过期 typed fail；无具体 Registry 旁路 | `cargo test --locked -p nomifun-app --features computer-use --lib computer_role_owner_runs_the_canonical_observe_input_chain -- --ignored --test-threads=1`；Computer serialization/generation/platform-unavailable tests；`cargo test --locked -p nomi-computer --lib -- --ignored --test-threads=1` | 本机 Windows Desktop/UI Automation 权限已通过；macOS 权限仍由外部主机验证 |
| `SL-S3-06` | closed | 主机 | 删除 Browser/Computer production concrete bypass | `SL-S3-04`、`SL-S3-05` | Wave 2 first-party Role owner 可用；Knowledge rendered source 只经 typed `browser.render_content`；Gateway Browser/Computer registry、capability module 和 standalone `mcp-computer-stdio` 已物理删除；Nomi engine 内部旧接线不得增长或成为消费者旁路，未来 Runtime 替换时再决定删除 | `cargo test --locked -p nomifun-knowledge --lib`（315）；`cargo test --locked -p nomifun-gateway --lib`（122）；`cargo test --locked -p nomifun-gateway --test production_bypass_audit`；`bun run check:browser-platform-boundary` | 无；旧 Gateway Browser/Computer 工具不再作为兼容入口提供 |
| `SL-S3-07` | closed | 主机 lane | 收口 Nomi-core 真实本地 owner | `SL-S2-06` | Chat、Workspace/File、Process、VCS、Knowledge search/read 保持真实调用；Coding 读写/patch/shell/diff/commit 接入 Nomi-core 的同一 Session 主链；非首批 Wave 3/4 不注册默认模板；不引入 Codex Sidecar 作为当前前置 | `powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\validation\run-nomi-core-live-provider-from-windows-credential-manager.ps1`；`cargo test --locked -p nomifun-app --test nomi_core_route_gap -- --test-threads=1`；`cargo test --locked -p nomi-tools vcs::tests --lib` | 2026-09-05 真实 StepFun Plan provider smoke 通过：Nomi-core Chat/Coding、Workspace/File、Process、VCS、同一 Session 的 Cron run/replay、Remote open/turn/observe/cancel、凭据持久化审计和关闭清理均完成；`live_smoke_status=pass code=OK status=200`。该次结果当时不替代独立 Desktop 验收；`SL-S4-02` 已在 2026-09-06 另行关闭 |
| `SL-S3-08` | closed | 主机 lane | 接入一个真实 MCP Tool 调用 | `SL-S2-06` | v4 `mcp_servers` identity、materialization、MCP package runtime catalog、exact tool/schema 和 credential authority 经 canonical capability；连接失败 typed fail；没有 Gateway/legacy fallback；owner 使用 no-redirect、bounded response 和一次 cleanup | `cargo test --locked -p nomifun-mcp --lib`（250）；`cargo test --locked -p nomifun-app --lib router::agent_wave2_mcp::tests -- --test-threads=1`（7）；`cargo test --locked -p nomifun-app --lib router::agent_wave2_host::tests -- --test-threads=1`（30） | 本机 disposable Streamable HTTP MCP fixture 已执行真实 `tools/call`；OAuth/stdio 仍明确 typed unavailable，不作为本项隐式扩张 |
| `SL-S3-09` | closed | 主机 | 实现精简 SSH read/write/exec/sudo owner primitive | 无 | 真实 host binding；最小 typed command/outcome；path/payload/output/timeout 有界；exec/sudo credential 分离；host-key changed fail；cancel 后回收且不自动重放 | `77bd45279`; `cargo check --locked -p nomi-ssh -p nomifun-ssh`; `cargo test --locked -p nomifun-ssh --lib` | live sshd/sudo 未运行时只记录未运行，不构造 PASS |
| `SL-S3-10` | closed | 主机 | 完成一个真实 scheduled/automation Nomi-core AgentSession | `SL-S2-05`、`SL-S2-07`、`SL-S3-07` | Schedule/Cron/AutoWork/Requirement 复用同一个 app-owned `NomiCoreSessionOwner` typed command/query 和 NomiCore runtime；计划、执行、取消、恢复不创建第二份 Conversation/Session identity；Channel/IDMM 的生产 crate 不再直接依赖 Conversation/Runtime registry，旧实现桥接只保留在测试支持或 app composition，receipt/scope 在 host 边界投影为领域自有类型 | `cargo test --locked -p nomifun-app --lib -- --test-threads=1`；`cargo test --locked -p nomifun-cron --lib --tests -- --test-threads=1`；`cargo test --locked -p nomifun-requirement --lib --tests -- --test-threads=1`；`cargo test --locked -p nomifun-agent-execution --lib -- --test-threads=1`；`cargo test --locked -p nomifun-idmm --lib -- --test-threads=1`；`cargo test --locked -p nomifun-channel --tests -- --test-threads=1`；`cargo check --locked -p nomifun-channel -p nomifun-idmm -p nomifun-app`；`bun run check:automation-session-boundary` | 2026-09-05 真实 StepFun smoke 已证明同一 Nomi-core Session 的 Cron run/replay/delete 可执行；审计 `scanned=194`、`production=175`、`tests=19`、`production_legacy_files=0`、`adapters=6`、`transitional_adapters_with_legacy_dependencies=0`、`test_compat_files=5`、`app_composition=6/6`、`candidate=none`。未来 canonical Session 直接 live event/完整 receipt/更宽 IDMM continuation 合同仍单独跟踪，不阻塞本项 |
| `SL-S3-11` | closed | 主机 | 闭合 Nomi-core Remote open/turn/observe/cancel 产品主链 | `SL-S2-03`、`SL-S3-07` | explicit AgentSession ID；binding/owner/provenance 不漂移；rotate/revoke 不挂起；cancel/delete/cursor/idempotency 明确；Remote 通过 NomiCore runtime，不依赖 Codex Sidecar；无最近会话或旧 selector 旁路 | `cargo test --locked -p nomifun-app --test nomi_core_route_gap -- --test-threads=1`；`cargo test --locked -p nomifun-public --lib`；真实 StepFun REST/MCP smoke | Nomi-core Remote REST 已支持 installation Bearer，并保留 owner JWT/local-trust 兼容；默认 `/mcp` 已接入公共 Streamable HTTP transport；真实 smoke 已通过 REST 与 MCP 的 `initialize/tools/list/open/turn/observe/cancel`，并验证 revoke 后旧 token 拒绝、删除后的 Session 不复活；不依赖 Codex Sidecar |
| `SL-S3-12` | deferred | 后续阶段 | 保留 Codex app-server 协议/Sidecar 研究与未来 Runtime host 接入 | `SL-S2-10`、未来 Codex 立项 | 只有未来正式立项后，才验证 source/build、exact binary、Provider/Model、工具回调、历史、取消/删除和平台证据；本阶段不实现、不重试、不作为 Nomi-core 交付条件 | 已完成的 upstream spike、协议 fixture 和生命周期回归仅作研究证据 | 当前不需要 exact pinned Sidecar 或 live Codex credential；fixture、adapter、Broker smoke 不得升级为 Codex-native PASS |

## S4：产品 UI

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S4-01` | closed | 主机 lane | 把 Agent 编辑器收缩为产品语言 | `SL-S2-07`、`SL-S2-08` | canonical Agent 工作台已满足名称/用途、模型、能力明细与关闭/启动即用/可按需申请三态；只展示 required resource kinds，不在 Preset 中绑定 Workspace/Knowledge/Connector；支持保存、试用和用户 Agent 删除；内部 ID/digest/JSON 默认折叠；Save/Test 自动 Preview | Agent Settings focused suite、资源边界/删除交互测试、`bun run build:ui` 和 i18n 通过；全量 typecheck 仍有仓库级 React 19/Arco 基线 | Desktop 产品走查已由 `SL-S4-02` 关闭 |
| `SL-S4-02` | closed | 主机 | 关闭真实 Nomi-core Agent 用户流程 | `SL-S3-04`～`SL-S3-11`、`SL-S4-01`、纠偏后的 AP-3/AP-4 | 从模板创建；查看完整能力明细；修改三态并保存；删除用户 Agent；从工作台“使用 Agent”进入 Guid 并预选；在 pill bar 切换后创建会话；按能力声明在会话中选择 Workspace/Knowledge；Preset 会话锁定模型，普通 Nomi 保留模型选择；删除 Agent 后历史 Session 保留、新 Session 被拒绝；全程不要求用户填写 UUID/operation/raw JSON | 真实 Tauri Desktop WebView 走查；`bun test --cwd ui`；Conversation/App/Cron 回归；当前 StepFun smoke | 2026-09-06 桌面产品走查通过；手机模式不属于 `nomifun-desktop` 本期服务范围 |

## S5：三平台 Nomi-core RC（未来 C9 已延后）

| ID | 状态 | Owner | 目标 | 依赖 | 完成定义 | 最小测试 | 人工 / 外部输入 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `SL-S5-01` | closed | 主机 | Windows Desktop x64 完成 Nomi-core 核心候选 | S0-S4 除原生外部项 | package/install/fresh/launch；Nomi-core Chat/Coding/File/Process/VCS/MCP/Browser/Computer/Knowledge/automation/Remote；cancel/crash/process-tree cleanup；无 P0、数据损坏或 secret 泄漏 | `bun run gate:agent-v2 -- c8-win-pre` | 候选 `0bac72da4ebb62f6a0f183a1285065c88aa684a4` 已通过；Host/NSIS/release-lock digest 与安装 smoke 见上方闭合记录 |
| `SL-S5-02` | external | 外部 | macOS Desktop arm64 Nomi-core 候选 smoke | `SL-S5-01` | 真 Apple Silicon 上对当前 Nomi-core 候选完成 package/install/launch、critical capability、anchored FS 和 dispose；非 Rosetta；只验证冻结候选真实 bytes | macOS arm64 native gate/critical suite | 需要 Apple Silicon、签名/打包环境和对应 Nomi-core 候选；不要求 arm64 Codex Sidecar |
| `SL-S5-03` | external | 外部 | Linux Desktop x64 Nomi-core 候选 smoke | `SL-S5-01` | 真 Linux Desktop x64 上对当前 Nomi-core 候选完成 package/install/launch、Coding、MCP、Browser availability、dispose；Computer 按一期声明明确 available 或 unavailable | Linux Desktop native gate/critical suite | 需要真实 Linux Desktop x64 和 Nomi-core 候选；不要求 Linux Codex Sidecar |
| `SL-S5-04` | deferred | 后续阶段 | 未来 Codex 切换后的 C9 shutdown 与 Nomi 物理删除 | 未来 Codex 立项、source/build、替代 Runtime 证据 | 只有未来 Runtime 正式接替且用户确认后，才停止 Nomi admission、清理进程树、标记 uncertain、删除 Nomi runtime/factory/route/artifact 并运行 residual scan | 未来 `c9-hard-delete` 及对应 dependency scan | 当前不执行；Nomi 是本阶段产品内核，不得把此项作为当前阻塞 |
| `SL-S5-05` | blocked | 主机/外部验证 | 对同一 Nomi-core RC 完成三平台最终验证并原样提升 Stable | `SL-S5-01`～`SL-S5-03` | Windows、macOS arm64、Linux Desktop 对同一 Nomi-core RC bytes 完成 package/install/fresh/critical E2E/lifecycle；release lock/result 可追溯；Stable 不重建；不依赖 C9 或 Codex Sidecar | 三平台 Nomi-core final RC suite；Artifact digest exact-match | 需要当前 Windows 环境、两个对应真实原生环境和发布/签名负责人执行最终 promotion；未来 Nomi-free RC 另行建项 |

## 当前 Runtime 组合与主机并发安排

### 默认产品入口

当前三类入口都选择同一个 Nomi-core 产品组合：

| 入口 | 当前组合 | 说明 |
| --- | --- | --- |
| Web | `NomiCoreApplication` | 运行原有 Nomi engine；不是 Fresh-v4/Codex host |
| Desktop `start_with_outcome` | `NomiCoreApplication` | 使用桌面本地 trust policy，但执行图仍是 Nomi-core |
| `nomicore` 默认服务入口 | `NomiCoreApplication` | 作为统一 in-process Nomi-core server |

`FreshV4Application` 仍保留为显式的低成本 host boundary，供二期/三期重新接入
Codex app-server 或其他 Runtime。当前没有运行中 Runtime selector、per-turn 切换或
自动 fallback；既有 AgentSession 不能在两个 host 之间迁移。

已完成并记录定向 evidence 的本机基础 lane：

1. `SL-S2-05` SessionEvent/Projection 收缩；
2. `SL-S2-06` 三类 Effect 策略收缩；
3. `SL-S2-07` canonical Compiler；
4. `SL-S2-08` small Snapshot/Fresh-v4 投影；
5. `SL-S2-09` PluginRegistration；
6. `SL-S2-10` official app-server upstream spike；
7. `SL-S3-09` 精简 SSH owner。

当前可实施或可并行准备的本机 lane：

- Core owner lane：`SL-S3-07` 已关闭；后续只处理真实 smoke 暴露的确定性缺陷，中央 composition
  由集成 Owner 收口；
- Automation lane：`SL-S3-10` 已关闭；Cron/AutoWork/Requirement/AgentExecution/
  Channel/IDMM 的 host-owned typed boundary 已收口，后续只维护确定性缺陷；
- Remote lane：`SL-S3-11` 已关闭；Nomi-core `open/turn/observe/cancel` 与 REST/MCP
  transport 只维护确定性缺陷；
- MCP lane：`SL-S3-08` 已关闭；后续 OAuth/stdio 扩展不进入本期隐式范围；
- UI lane：`SL-S4-01` 已关闭；仓库级 React/Arco 类型基线不反向阻塞已通过定向验收的
  Agent Settings 产品切片；
- Role Provider lane：`SL-S3-04`、`SL-S3-05` 已关闭；后续只维护真实平台回归；
- Codex Sidecar lane：`SL-S3-12` 已延后；只保留协议研究和未来接入前置条件，不占用当前
  Nomi-core 关键路径。

### 2026-09-03 本机 checkpoint

- Runtime 组合：`apps/web`、Desktop `start_with_outcome` 和 `nomicore` 默认入口均已
  选择 `NomiCoreApplication`；`FreshV4Application` 仅作为显式备用 host boundary
  保留。当前不存在运行中 Runtime 切换或自动 fallback。
- Rust/App 主线：`cargo test --locked -p nomifun-app --lib -- --test-threads=1`
  `380 passed`；启动 smoke `5 passed`；Kernel `18 passed`；Platform sample
  `2 passed`；Fresh-v4 root `11 passed`。
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
- Chat/Coding live：新增有界的生产 Broker smoke，临时创建 Step Plan provider/model/
  capability/route 图并只从受控输入读取凭据。2026-09-03 首次代理请求曾返回
  `ProviderUnavailable`、HTTP `503`；该配置问题已修正，2026-09-05 通过 Windows
  Credential Manager runner 完成真实 StepFun smoke，结果为
  `live_smoke_status=pass code=OK status=200`。凭据没有进入源码、报告、命令参数、
  Cargo/build、测试二进制或应用子进程环境，也没有进入仓库；仅短暂存在于受控
  runner 进程内。
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
- Automation 审计确认 Cron、AutoWork、AgentExecution 生产消费者仍经
  `ConversationService`/Conversation-backed compatibility port；Nomi engine 本身是当前
  产品执行内核，不能把 typed delegator 单测当作 canonical Session migration close。
- Session receipt：`AgentSessionStore::read_turn_receipt` 已在同一只读事务内按
  `(AgentSessionId, OperationId)` 返回 `running/completed/failed/cancelled/not_found`，
  并穿过 `AgentSessionQueryPort`；同一 operation 的首个终态具备单调 fence，late start
  和冲突终态被拒绝。Session crate `23 passed`，AgentPlatform `15 passed`，
  chat-minimal `2 passed`。
- AgentExecution：scheduler、planning、outbox retry 和 lease heartbeat 共用一个
  composition lifecycle；关闭前先取消并等待这些后台任务，避免数据库关闭后的残留查询。
  AgentExecution crate `87 passed`。
- Cron/AutoWork：Cron 现在只接受 typed `PublicTurnDeliveryState` 的 terminal receipt；
  text/Finish/idle、receipt 丢失、持续 probe/reconcile 错误和 accepted 等待均有
  fail-closed 或 hard deadline；`Ok(None)` claim verdict 不再被当成成功。
  Cron `197 passed`，Requirement `102 passed`。
- Remote：REST 每个 lookup/admission/dispatch/page/cancel 路径都有独立 deadline；
  mutation 超时保留固定容量的 detached 后台收敛并要求复用同一 idempotency key，
  不能把未知结果改写成成功；关闭时停止新 admission 并有界等待。Remote REST 单测
  `7 passed`、Runtime 单测 `3 passed`、Remote E2E `2 passed`。
- Sidecar：opening 卡死 fixture 在 bounded timeout 后清理 opening registry 和进程树；
  Codex Runtime `33 passed`。这只是未来 host 的协议/生命周期 research evidence，不替代
  exact source build、binary、live model，也不阻塞当前 Nomi-core 交付。
- Windows 启动：普通 PowerShell 不再要求预先进入 VS Developer Prompt；`bun run dev`
  会在子进程内定位 VS Build Tools/Windows SDK，desktop 首次完整链接成功。真实 Tauri
  WebView `/guid` smoke 页面正常，console/pageerror 均为空；外部 Chrome 直连因没有
  Tauri local-trust 注入而得到 403，已归类为错误 harness。
- 测试障碍：一次并发热重载期间曾出现 Rust 编译器 `0xc0000005`；停止并发写入后
  单独启动成功。Cron integration 曾出现 Windows 临时技能目录
  `PermissionDenied (OS 5)`，Playwright 本地模块也出现 ESM/CJS export 不兼容；
  应用本身的 Tauri/后端 smoke 已完成，以上均作为环境/harness 记录，未继续盲目重试。
- 关闭生命周期：Nomi-core 应用和桌面 host 先取消后台任务，再清理 Terminal、Gateway/
  Browser、Robot、SSH，最后关闭数据库；IDMM janitor 不再在数据库关闭后继续查询。
- 默认所有开发、合流和验证继续在本机进行，不建立长期跨机开发协议。2026-09-09 用户
  明确要求将 Windows N1/M1 阶段性交给另一台机器；启动上下文集中记录在
  `CROSS-MACHINE-WORK-START-PROMPT-2026-09-09.zh.md`，不产生压缩包、跨机 attestation
  或第二个状态源。

### 2026-09-05 本机续接 checkpoint

- Remote 本地闭环复核通过：`cargo check --locked -p nomifun-app --lib -p
  nomifun-db`、Remote repository `1 passed`、Nomi-core route gap `7 passed`、Remote
  REST `7 passed`、取消错误 `2 passed`、启动 smoke `5 passed`。阶段转移与事件追加
  仍在同一 SQLite 事务；相同 operation key 的 payload 冲突不会被当作 replay；terminal
  outcome 会吸收迟到完成；Remote open/turn/cancel 返回 Remote event cursor。
- AutoWork 生命周期复核通过：`cargo test --locked -p nomifun-requirement --lib --
  --test-threads=1` 为 `115 passed`，`--tests` 合计 `120 passed`。sweeper、boot
  resume 和 active target loop 均有 cancellation/join 或 exact claim cleanup；
  关闭不会在数据库关闭后继续运行这些任务。
- UI 收口复核通过：`bun test --cwd ui src/renderer/pages/agentSession
  src/renderer/pages/agentSettings` 为 `20 passed`，`bun run build:ui` 和
  `bun run check:i18n` 通过。AgentSession 结构测试已改为检查实际渲染 generation 的
  `SessionInspector` 与共享错误分类器，不再依赖已移动的页面字面量。
- Coding/on-demand 继续 fail-closed：当前 Nomi-core 没有 canonical activation port
  时不把 on-demand 能力伪装为已启用；真实 Coding smoke 使用 initial capabilities，
  并同时回归验证 on-demand placement 会显式阻断。2026-09-05 真实 StepFun
  Nomi-core Chat/Coding 写入语义已通过；这只直接关闭 `SL-S3-07`，不改变
  on-demand activation 或 Desktop 人工验收的独立完成定义；`SL-S3-10` 和
  `SL-S3-11` 另有各自的本机合同证据。
- `SL-S3-10` 已完成 host-owned typed boundary 收口：最新
  `bun run check:automation-session-boundary` 扫描 `scanned=194`、
  `production=175`、`tests=19`，`production_legacy_files=0`、`adapters=6`、
  `transitional_adapters_with_legacy_dependencies=0`、`test_compat_files=5`、
  `app_composition=6/6`，并报告 `candidate=none (status=complete)`。
  Cron、Requirement/AutoWork、AgentExecution、Channel、Companion 和 IDMM 的生产
  消费者均经同一个 `NomiCoreSessionOwner` 组合；Channel 的 Conversation bridge
  已移到 integration-test support，IDMM 的 scope/admission 转换已移到 app
  composition，生产 crate 不再直接依赖旧 Conversation/Runtime 实现。
  未来 canonical Session 的直接 live event、完整 Channel receipt 和更宽的 IDMM
  continuation/failover surface 仍是独立后续合同，不被本项伪装为已完成。
- 本轮主机接线已补齐：Cron 使用封闭 `CronTurnRuntimeOverlay`、原子双侧关系绑定和
  typed receipt/reconciliation；AutoWork 使用 host-owned opaque lease issuer、
  Session projection revision fence、owner/revision/operation-aware 配置 CAS；
  Companion archive 使用 `NomiCoreSessionOwner` 的 owner-scoped window/reset
  contract；Channel 使用 Channel-owned receipt projection；IDMM 的 scope/admission
  核心改用 IDMM-owned typed contract，并由 app composition 负责 host token 转换。
- SQLite `conversation.extra` CAS 已有顺序 stale-writer 与跨 repository handle
  并发 winner 测试。该 CAS 只更新 AutoWork-owned metadata 并保留其他 extra 字段，
  不改变 Conversation/AgentSession 的唯一身份。
- 安全 live Provider runner 已通过 `--self-test` 与 `--compile-only`：
  `live_smoke_runner_self_test_status=pass code=OK status=200`、
  `live_smoke_compile_status=pass code=OK status=200`；随后用
  `powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File
  .\scripts\validation\run-nomi-core-live-provider-from-windows-credential-manager.ps1`
  完成真实请求，结果为 `live_smoke_status=pass code=OK status=200`。runner 从
  Windows Credential Manager 读取 secret，并在 Cargo/build 完成后只向测试进程 stdin
  发送一次；runner 在启动 Cargo/build、测试二进制和应用子进程前剥离环境变量，
  应用关闭后完成明文持久化审计。
- `SL-S3-11` 已关闭：Nomi-core Remote REST 已补 installation Bearer authentication，
  保留 owner JWT/local-trust 兼容，并通过无 local-trust 与旧 selector query 回归；
  默认 `/mcp` 已接入公共 Streamable HTTP transport。真实 smoke 覆盖 REST 与 MCP
  的 `initialize/tools/list/open/turn/observe/cancel`，并验证 revoke、cursor、
  idempotency 和 delete 后不可复活。
- 截至本 checkpoint，真实 Provider、`SL-S3-10` automation、`SL-S4-02` Desktop
  走查与 `SL-S5-01` Windows candidate 均已关闭。macOS arm64/Linux Desktop x64
  原生验证保留，但按用户要求延后到 06 的 Windows Plugin/MiniApp 开发全部完成后
  统一交接。不得用 mock、fixture、静态 adapter 或 synthetic PASS 关闭这些项目。

所有 lane 都在当前主机使用互斥路径写集；不得同时编辑同一文件或争用共享数据库、固定
端口、Cargo 构建目录和进程树。`Cargo.lock`、中央 Compiler/Snapshot、Fresh-v4 schema、
Gate 和 GLOBAL TODO 由集成 Owner 串行修改。每个 lane 只记录 changed paths、验证命令、
未运行项和阻塞原因，不生成机器专用 Prompt、manifest、result template、远端 SHA、
handoff 或跨机 attestation。

## 推荐顺序

1. 保留 AgentPreset 能力边界、模型冻结、Snapshot 资源种类、Cron resolver 和
   Desktop 产品流程的干净实现提交
   `ef5f5380915e0d5c06004f03b66cbd1302b3fe03` 与 AP-7 signed evidence。
   `SL-S3-07`、`SL-S3-10`、`SL-S3-11`、`SL-S4-02` 已关闭；
   `NomiCoreSessionOwner` 的 Cron/AutoWork/Companion archive/Channel/IDMM typed
   边界进入维护状态，除确定性合同缺口外不扩大兼容层。
2. 保留 `0bac72da4ebb62f6a0f183a1285065c88aa684a4` 的 Windows C8 结果作为一期
   工程闭合证据，不在其上混入 06 代码。
3. 按 `06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md` 和独立
   N1/M1 台账，先完成 Plugin Windows，再完成 MiniApp Windows。
4. 06 的全部 Windows 开发和候选验证完成后，冻结新的最终 source cohort，再统一交给
   macOS arm64 与 Linux Desktop x64 做原生验证；发现问题返回当前主机修复。
5. Codex Sidecar、C9/Nomi-free RC 保留为后续阶段，不进入 N1/M1 当前关键路径。

## 阶段退出条件

- **S0 完成**：本文审查提交，单机多并发规则生效，旧 84 项不再驱动施工。
- **S1 完成**：Wave 3/4 都有明确 keep/revert 结果，保留代码都有真实用户价值。
- **S2 完成**：三个 P0 关闭；单 Compiler、小 Snapshot、三类 Effect 生效；NomiCoreApplication
  成为当前统一产品组合；Codex upstream spike 仅作为未来 host boundary 研究。
- **S3 完成**：Browser/Computer first-party 与 alternate fixture 走同一 Role seam；核心用户
  owner、MCP、SSH、automation、Remote 经 Nomi-core Session 主链真实执行；不产生 Codex
  Sidecar 旁路或运行时 fallback。
- **`SL-S3-10` 收口**：六类 Session consumer 的生产依赖审计无旧实现引用；
  Cron/AutoWork/Requirement/AgentExecution/Channel/IDMM 均通过同一个
  `NomiCoreSessionOwner` 接收 typed host contract，测试兼容桥只存在于测试支持目录，
  且不把未来 canonical Session 的直接 live event 或扩展 continuation surface 宣称为已交付。
- **S4 完成**：桌面用户流程可从真实 Tauri Desktop 正常验收，普通用户不接触内部标识和
  JSON；手机模式不属于 `nomifun-desktop` 本期服务范围。
- **S5 完成**：Windows、macOS arm64、Linux Desktop 的同一 Nomi-core RC 通过，具备
  当前阶段 Stable 提升条件；不要求当前阶段删除 Nomi。

macOS x64、Linux Headless、Wave 3/4 非核心业务全覆盖、Knowledge 高级写入/embedding/rerank、
所有 Channel/Robot/Customer 场景、性能 benchmark 和长观察窗口不阻塞首个 Stable。未来实际
宣称支持相应平台或功能时，再建立独立、真实、可验证的交付任务。

Codex-native、external Codex integration、C9 shutdown 和 Nomi-free RC 不是当前阶段的
未完成阻断项；它们只有在后续重新立项并满足独立前置条件后，才建立新的执行台账。

## 2026-09-08 06 Windows 主线状态同步（M1-1 收口后）

AP-0～AP-7、一期 C8 与 S0-S5 的口径保持不变；06 的实时状态继续由
PHASE-N1-M1-CLOSURE-TODO.zh.md 维护。M1-0-02-A/B、M1-1-01 和 M1-1-02 已完成
Windows 主机实现与定向回归：真实 dedicated Service Host、Service Storage IPC、
owner-scoped Files、Host-managed Private SQLite、authorizer、Migration ledger、
Publish migration fence 和 Runtime candidate Service 验证均已接入并推送。

当前下一步是 M1-2 生命周期/Service Test/导入导出实现；Windows Candidate/NSIS、
最终 cohort、macOS/Linux 原生验证和 Stable 提升仍未关闭。手机模式不属于
`nomifun-desktop` 范围。

## 2026-09-08 06 Windows 主线状态同步（M1-2 生命周期删除子切片）

`86afa7af6` 已完成 M1 Trash/Restore/Permanent Delete、失败 Retry 与启动 Reconciler：
deleting intent 和不可取消 Operation 使用 owner-scoped SQLite transaction；Catalog/
Surface 先撤销，Service、Files、Private SQLite、Source、Release 和数据库 owner rows
从头幂等清理。Windows purge 已加入父链 canonical containment 与 junction/reparse 拒绝；
Desktop Workshop 已接四个生命周期动作、删除进度和失败恢复。

该 checkpoint 通过 DB 30、Platform 17、App route 6、UI/wire 26 项定向测试，以及受影响
crate check、i18n、rustfmt 和 diff check。`M1-2-01` 仍为进行中；下一步依次完成 Service
Test transient namespace/receipt、Share Bundle/prebuilt Import、disabled Whole-App Backup
Import-as-new，再进入 MiniApp Capability Catalog、旧 MiniApp 拆除和 Windows Candidate。
macOS/Linux 继续等待最终 Windows cohort，手机模式不在范围内。

## 2026-09-09 06 Windows 主线状态同步（M1-2 Service Test）

`708ef83b7` 已交付 Service Test transient KV/Private SQLite/empty Files、Ready Migration、
独立 one-shot Node Host、Host receipt、Runtime/revision stale 判定和 Desktop Test 动作。
真实 Node Test Host、重启清理、并行 namespace 与 Windows junction 拒绝均已验证。

`M1-2-01` 继续为进行中；下一步是 Share Bundle/prebuilt Import 与 disabled Whole-App
Backup Import-as-new。随后再进入 MiniApp Catalog 正式消费、旧 MiniApp 拆除和 Windows
Candidate；macOS/Linux 与手机范围不变。

## 2026-09-09 06 Windows 主线状态同步（M1-2 Share Application）

Share Application/API/E2E 已关闭；Desktop Share UI 正在实施，Whole-App Backup 尚未
开始，`M1-2-01` 继续为 `in-progress`。完成顺序为 Share UI → Whole-App Backup →
MiniApp Catalog 正式消费 → 旧 MiniApp 拆除 → Windows Candidate。macOS/Linux 继续等待
最终 Windows cohort，手机模式不在范围内。

## 2026-09-09 06 Windows 主线状态同步（M1-2 Backup 收口）

MiniApp M1-2 的 Share/Prebuilt Import、Desktop Transfer UI 和 disabled Whole-App
Backup Export/Import-as-new 已完成 Windows 定向实现与回归。Backup application 已覆盖
UI-only 以及 Service Files、Private SQLite、Migration ledger、Release 重绑定和新
identity Catalog digest；普通 Share Export 与 Backup Export 的 owner operation 互斥也
已验证。对应 `M1-2-01` 在 06 台账中关闭。

本 checkpoint 不关闭 `M1-U-01`、`M1-V-01` 或 `RC-WIN-01`。下一顺序保持：
MiniApp Capability Catalog 正式 consumer integration → 旧 MiniApp 生产链物理清理
→ Windows Candidate/NSIS/fault/accessibility → 最终 Windows cohort；只有其后才交接
macOS arm64 与 Linux Desktop x64，手机模式不属于 `nomifun-desktop` 范围。

## 2026-09-09 Windows N1/M1 阶段性交接

用户要求先进行阶段性收尾，再由另一台更快的 coding agent 接手后续 Windows 主线。
当前交接材料为：

`CROSS-MACHINE-WORK-START-PROMPT-2026-09-09.zh.md`

该文档固定了 `rf/agent-capability-platform-v2` 的交接基线
`51b0243f7587df22a4007ad63655b64d0f861b7a`、已交付的 M1-2 Whole-App Backup
闭环、当前未关闭边界、第一轮 MiniApp shared Catalog consumer integration 的启动方法、
Windows/Provider/Git 安全规则和停止边界。它只用于跨机器启动，不得覆盖本文状态，
不得恢复旧 Catalog 实验、旧 Extension/MiniApp 兼容路径或已撤销的跨机执行协议。
