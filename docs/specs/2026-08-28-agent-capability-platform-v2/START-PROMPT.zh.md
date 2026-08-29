# Agent Capability Platform v2 重构任务启动 Prompt

## 使用方式

在任意电脑上先取得本重构分支，并从仓库根目录启动任务：

```bash
git fetch --prune origin
git switch --track origin/rf/agent-capability-platform-v2
```

如果本地已经存在该分支，则使用：

```bash
git switch rf/agent-capability-platform-v2
git pull --ff-only origin rf/agent-capability-platform-v2
```

确认当前工作区中的用户改动已经得到保留后，把下面整段 Prompt 交给新的 Codex 任务。仓库所在目录名可以任意；所有路径都从当前仓库根或兄弟仓相对解析。

## 可直接复制的启动 Prompt

```text
你现在负责正式实施 NomiFun Agent Capability Platform v2 全量重构。

当前 Git 分支必须是：

rf/agent-capability-platform-v2

当前仓库目录名和绝对路径不固定。只能以当前 Git 仓库根为基准解析本仓文件；参考源码仓使用：

- ../codex
- ../deepseek-harness

如果兄弟仓不存在、不是预期仓库或基线提交不符合调查文档，先完成只读核验并明确报告，不得偷偷改用另一份代码或机器绝对路径。

一、开始前必须完成的读取与状态检查

1. 完整阅读仓库根 AGENTS.md 及当前作用域内所有 AGENTS.md。
2. 按顺序完整阅读，不得只看摘要：
   - docs/specs/2026-08-28-agent-capability-platform-v2/README.zh.md
   - docs/specs/2026-08-28-agent-capability-platform-v2/DECISIONS.zh.md
   - docs/specs/2026-08-28-agent-capability-platform-v2/01-current-state-and-harness-findings.zh.md
   - docs/specs/2026-08-28-agent-capability-platform-v2/02-capability-catalog-and-agent-presets.zh.md
   - docs/specs/2026-08-28-agent-capability-platform-v2/03-target-architecture.zh.md
   - docs/specs/2026-08-28-agent-capability-platform-v2/04-migration-and-validation-plan.zh.md
3. 检查当前 branch、HEAD、origin、worktree、用户未提交改动、Git identity、兄弟仓状态和远端分支 SHA。保留并绕开不属于本任务的改动，不得 reset --hard、强制清理、force-push 或改写共享历史。
4. 检查 docs/specs/2026-08-28-agent-capability-platform-v2/IMPLEMENTATION-STATUS.zh.md：
   - 若不存在，在首个实施提交中创建；
   - 若存在，从其中记录的 final remote SHA、当前 C0～C11 节点、已关闭 slice、验证证据、未完成 write set 和下一可执行任务继续；
   - 不得重新完成已经有 commit/evidence 证明闭合的工作。

二、任务性质与总目标

设计阶段已经结束，D-001～D-028（含 D-019）全部确认。不要重新发散调研、重新提出 A/B/C 方案，也不要要求用户再次确认已经写入 DECISIONS.zh.md 的内容。你的任务是按照冻结方案实施、验证、分阶段提交并持续推送本分支。

最终目标包括但不限于：

1. 用 Codex-derived Runtime 完整替换不成熟的 Nomi Agent Runtime，尽可能完整保留 Codex 围绕 Coding 的能力，不得把 coding.codex-native 降级成普通问答或通用 Tool 组合。
2. 建立 Thin Functional Kernel、trusted in-process Package、Capability Registry、AgentPreset Compiler、Plugin Manager 和可组合的系统能力插件架构。
3. 将 Knowledge、Memory、Companion、Browser、Computer、IM/Channel、Customer Service、Robot、Creative Studio、Requirement、Auto Work、Cron、IDMM、AgentExecution、SSH、Office、Webhook 等业务能力从 God Service/Factory/Gateway 手工装配中解耦为插件贡献。
4. 建立 AgentPreset 原子能力组合、initial/on-demand 投影、typed resource binding、Capability Catalog 和未来第三方插件挂载缝；本期不实施 Phase N 第三方安装市场。
5. 新架构唯一会话事实为 AgentSession/AgentSessionId；API 使用 agent-sessions，数据库使用 agent_sessions，彻底删除新架构中的 Conversation 技术术语和双身份映射。
6. 只保留 FullAuto/YOLO：AskForApproval::Never + DangerFullAccess。删除 default/auto_edit/审批/确认/permission-review/wait 等产品模式和状态机，同时完整保留 Coding 的代码审查、diff review 与 review workflow 能力。
7. 使用 fresh-v4 clean start，不迁移旧数据、不开发 converter、不建立兼容层、旧 Runtime fallback、双写或 data downgrade。
8. 按迁移 slice 同改同删旧 Nomi、Factory、GatewayDeps、AppServices、旧 API/DTO/table/config/test/dependency；禁止先堆新链、以后再建 cleanup backlog。
9. 官方模板 exact-set 只有七个：chat.minimal、assistant.general、coding.codex、companion.default、robot.default、customer-service.default、creative-studio.default。Research 是 research.core Capability Pack；research.web、requirements.analyst、autowork.executor 是必须删除的 legacy Preset key。
10. 完成 Nomi physical hard delete、五格原生 RC 验证和 same-digest Stable Gate；不得把“代码写完”表述成“正式发布完成”。

三、不可更改的实施原则

1. 交付速度和结构简单优先。安全平台、sandbox、WASI、签名市场、复杂权限系统和 Agent 审批模式不是本期目标；必要的数据归属、typed binding 和 RuntimeAuthority 校验按冻结合同实现，不得借安全名义扩张范围。
2. 不受历史架构和重构成本约束。发现僵尸代码、过时设计、循环依赖和重复能力时，按目标架构删除或重构，不打兼容补丁。
3. 轻量 Preset 必须从空集合正向构造；chat.minimal 最终 tools=[] 且无隐藏初始化。Coding Preset 必须保留完整 Codex-native 能力。
4. Package、Capability、Skill、MCP 是四个真实概念；只有 Capability 进入 Agent 能力组合主链。内部 Rust service 仅使用轻量 typed ServiceKey 接线，不创建第五类产品对象。
5. PlatformValidationManifest、PlatformCellEvidence、verification ledger 和 recheck 只是 repo-local engineering artifacts，不进入产品数据库、API、UI、Preset、SessionEvent、审批或自动化状态机。
6. Canonical cohort tuple 为：
   candidate_source_sha / confirmed_decision_contract_digest / platform_validation_manifest_digest / runtime_release_digest
   任一字段变化都按影响规则复验，只有四字段 exact-equal 才能沿用旧证据。
7. Tuple digest 必须使用无自引用单向链：immutable CodexRuntimeReleaseManifest input → runtime_release_digest → immutable PlatformValidationManifest input → platform_validation_manifest_digest → native evidence → 独立 post-run summary/envelope。Merge 不得回写 input manifests。

四、并行实施与所有权

默认使用 6～8 个高并发 coding agents，但必须先给每个任务写出 disjoint write manifest。五条稳定 owner workstream 为：

- W1 Platform Foundation & Fresh-v4
- W2 Codex Runtime & Providers
- W3 Product Control Plane
- W4 Domain Migration & Inline Demolition
- W5 Shared Integration, Hard Delete & Release

执行要求：

1. 同一文件、Rust module、migration、schema、Event Registry、Cargo workspace、共享 fixture、Composition Root、UI route 或 release Gate 同时只能有一个 writer。
2. W4 在 C6 后最多拆三个临时 Domain pods；pod 不得直接修改 shared schema/Composition/Cargo/Gate 文件，只能交给 W1/W5 integration owner 串行合入。
3. 子 Agent 先做相互独立的 bounded slice；主 Agent 负责依赖图、中央文件、冲突处理、验证去重、提交和远端状态。
4. 每个 closed slice 的完成定义必须同时包含 canonical producer、全部 direct consumers、同改同删、targeted/fault evidence 和 residual/reachability closure。
5. 不要让多个 Agent 同时运行 workspace Cargo build/test。日常只运行最小 targeted checks。

五、唯一实施顺序

从 Review A / Contract Closure 与 C0/G0 开始，不得跳过 G0 直接编写 production migration/seed：

1. C0 / G0：冻结 canonical contracts、fresh-v4 schema、SessionEvent vocabulary、D-014 manifests、OfficialPresetSeedManifest、D-025～D-028 fixtures、PlatformValidation input schemas 和 digest。G0 不实现 production behavior。
2. C1：物理删除 FullAuto 之外的模式、审批和等待主链。
3. C2～C5：在 C1 后按 disjoint paths 并行完成 Fresh-v4、Kernel/Plugin core、Codex Runtime/Providers、Preset Product。
4. C6：Chat + Coding + sample.echo 三联 final-stack Gate。
5. C7：Domain slices；每个 slice 同时切 consumer、完成新链、删除对应 Nomi/legacy wiring。
6. C8-WIN-PRE：只有 Windows 上 C1～C7 全部功能开发、业务域、UI、跨平台代码预留和中央集成完成后，才生成完整 Windows pre candidate 并执行 Windows 全功能/pre-version Gate。
7. HP-1：C8-WIN-PRE 整体通过后才 commit、push、验证远端 SHA、生成 handoff bundle并暂停通知用户切换真实 macOS ARM64。
8. C8-MA：对整个候选完成 macOS ARM64 适配与 full native Gate，不按单功能暂停。
9. HP-2：C8-MA 整体通过后才 commit、push、验证远端 SHA，并通知用户在其他电脑并行启动 macOS x64、Linux Desktop x64、Linux Headless x64；canonical tuple 相对 Windows 任一字段变化时，同批包含 Windows full/scoped recheck。
10. C8-MX/C8-LD/C8-LH：三个 whole-candidate native tasks 并行。
11. C8-RECHECK-n：上一整轮五格全部返回后，才一次合入整批 fixes、冻结新 tuple并启动 whole-cohort recheck；affected cells 跑完整受影响 Gate，unaffected cells 在原生 Host 跑 scoped attestation。需要换机器时只在整轮边界一次提醒，绝不按单功能、单失败、单修复换平台。
12. C8-MERGE：五格同 tuple pass、pending/fail/stale=0 后，执行 D-027 final drain/zero，再进入 C9。
13. C9：物理删除剩余 Nomi source/feature/crate/package/dependency/test/reachability；此后不得通过 revert 恢复 Nomi，只能 forward fix。
14. C10：五格原生 Nomi-free RC 验证。RC fixes 同样等整轮结束后批量合入，使用 C10-RECHECK-n whole-cohort 收敛；C10-MERGE 同 tuple 全绿后才可 C11。
15. C11：只提升 C10-MERGE 已验证的相同 signed content digest，不重建另一份 Stable 制品。

如果任务从非 Windows 电脑首次启动，可以完成 host-independent 的只读 inventory、Contract Closure/G0 和与当前节点相符的独立工作，但不得宣称完成 Windows C1～C8-WIN-PRE。严格根据 IMPLEMENTATION-STATUS.zh.md 和真实原生 evidence 判断从哪里继续。

六、验证与效率要求

1. L0/L1：每个 slice 只运行 git diff --check、精确 rg/residual、受影响 crate/UI 的 format/check/test、必要 schema/route/typecheck。
2. workspace cargo test 只属于 C6、C8-WIN-PRE、C10-WIN 三个 Gate 节点族，由 validation coordinator 按 exact input tuple 去重；同一 tuple 只执行一次。整批修复生成新 tuple且使 Windows broad evidence stale 时，才在原节点族合并重跑。
3. macOS/Linux native cells 只运行 target-specific build/package/hello/Coding/lifecycle/fault/process checks；cross-compile、静态检查、WSL、容器、VM、模拟器、Rosetta 只能算 preflight，不能产生目标 cell PASS。
4. 高耗时检查失败时先由 owning slice 用最小复现定位，不要让所有 Agent重复运行全仓命令。
5. 文档或合同变更只运行与其风险匹配的机械检查；跨层实现、hard delete 和 RC 节点再运行相应 broad Gate。

七、Commit、Push 与跨机器连续性

1. 每个闭合 slice/阶段形成小而完整的 commit；禁止把“新增新主链”和“删除旧链”拆成长期分离的提交。
2. 提交前检查 git status --short、git diff --check、git diff --cached --name-status、write manifest、deletion manifest 和对应 evidence。
3. 使用当前贡献者配置的 Git identity，不添加额外署名或 AI trailer。
4. 只向 rf/agent-capability-platform-v2 普通 push。每次 push 前 fetch 并确认远端没有未知推进；禁止 force-push、历史重写或直接推 main。
5. 每个阶段 push 后核对 local HEAD、origin/rf/agent-capability-platform-v2 与 git ls-remote 返回的 SHA 完全一致。
6. 持续更新 IMPLEMENTATION-STATUS.zh.md，至少记录：
   - branch、base SHA、current SHA、last verified remote SHA；
   - 当前 C0～C11 节点和 workstream owner；
   - 已关闭 slices/commits/evidence；
   - active disjoint write sets 与中央文件队列；
   - 当前 canonical cohort tuple；
   - pending PlatformVerificationPoints、affected cells、原生 Host/recheck 状态；
   - 已运行/未运行的验证和原因；
   - 下一批可直接执行的任务与真实 blocker。

八、自主执行与停止条件

不要只交付计划或再次总结设计。完成只读核验后立即从当前节点开始实施，并在当前平台阶段内持续推进。

只有以下情况可以停止并请求用户：

1. 到达 HP-1、HP-2 或 whole-cohort recheck，需要用户切换/准备真实目标机器；
2. 发现会改变已确认产品语义、required platform exact-set 或 gross scope 的新事实；
3. 需要用户提供当前环境无法取得的外部凭据、硬件、签名材料或真实业务资源；
4. 同一个外部阻塞经充分排查后仍无法推进。

普通编译错误、测试失败、代码冲突、重构困难、shared fix、目标平台适配问题或需要更多实现工作都不是停止理由。先在当前整候选批次内集中修复、验证、提交并更新状态。

进度更新必须简短明确，区分：已完成代码、已通过验证、待原生验证、内部 QA 可交付、Stable Gate 已通过；不得把静态检查、cross-compile 或未签名包描述为正式发布完成。

现在开始：先检查仓库/分支/状态文档，完整读取六份规范，建立 C0/G0 的 disjoint work plan 与 IMPLEMENTATION-STATUS.zh.md，然后立即执行当前可落地的 Contract Closure/G0 工作。除非命中上述停止条件，不要等待新的用户确认。
```
