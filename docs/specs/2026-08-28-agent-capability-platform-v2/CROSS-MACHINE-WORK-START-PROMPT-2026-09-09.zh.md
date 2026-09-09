# Windows N1/M1 跨机器阶段性交接与工作启动 Prompt

> 创建日期：2026-09-09
> 适用分支：`rf/agent-capability-platform-v2`
> 交接基线：`51b0243f7587df22a4007ad63655b64d0f861b7a`
> 适用范围：Windows Desktop x64；不包含手机模式，也不提前执行 macOS/Linux 原生验证

## 文档性质

这是用户明确要求生成的**一次性阶段性交接材料**，用于让另一台更快的 coding agent
从当前稳定基线继续 Windows N1/M1。它不是架构合同、状态源或新的跨机开发协议：

1. 产品与架构以 `05-system-capability-replacement-foundation.zh.md` 和
   `06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md` 为准；
2. 一期状态以 `GLOBAL-CLOSURE-TODO.zh.md` 为准；
3. N1/M1 实时状态以 `PHASE-N1-M1-CLOSURE-TODO.zh.md` 为准；
4. 本文只说明启动顺序、当前事实、下一步边界和交付纪律；与上述文档冲突时本文失效；
5. 不要从 Git 历史恢复旧 `START-PROMPT`、旧 handoff、旧跨机批次清单或已撤销的
   兼容方案。

## 可直接复制给下一台 coding agent 的启动 Prompt

```text
你现在接手 NomiFun Desktop 的 Windows N1/M1 阶段性重构。

一、先建立可靠基线

1. 进入仓库根目录后执行：
   git fetch origin
   git checkout rf/agent-capability-platform-v2
   git pull --ff-only
   git status --short --branch
   git rev-parse HEAD
2. 预期分支为 rf/agent-capability-platform-v2，当前交接基线为：
   51b0243f7587df22a4007ad63655b64d0f861b7a
   如果远端已有更新，以远端最新提交为准，但必须先阅读本交接文档和最新台账；
   不要 reset、force-push 或重写共享历史。
3. 依次阅读：
   - docs/specs/2026-08-28-agent-capability-platform-v2/05-system-capability-replacement-foundation.zh.md
   - docs/specs/2026-08-28-agent-capability-platform-v2/06-phase-n1-plugin-miniapp-simplified-implementation-plan.zh.md
   - docs/specs/2026-08-28-agent-capability-platform-v2/PHASE-N1-M1-CLOSURE-TODO.zh.md
   - docs/specs/2026-08-28-agent-capability-platform-v2/GLOBAL-CLOSURE-TODO.zh.md
   - 本文 CROSS-MACHINE-WORK-START-PROMPT-2026-09-09.zh.md
4. 先核对真实代码和台账，不要根据旧会话摘要或旧 Git 历史推断已完成。

二、当前目标

在 Windows Desktop x64 上完成 06 所描述的 N1/M1，不做过度设计：

1. 先把 MiniApp Active Release 的能力贡献接入现有共享 Formal Capability Catalog；
2. 再物理清理旧 MiniApp/Extension 的生产路由、启动加载和消费者依赖；
3. 完成 Windows Desktop 产品走查、accessibility、fault、NSIS 和 installed-app
   Candidate；
4. 只有 RC-WIN-01 关闭后，才把同一最终 Windows cohort 交给 macOS arm64 和
   Linux Desktop x64。手机模式不属于 nomifun-desktop 的范围。

三、最重要的设计边界

1. 不建立第二套 Agent 专属 Catalog，不复制 Plugin Catalog，不新增
   `contributes.presets`，不让 Package 拥有 AgentPreset。
2. `N1-3-01/02/03` 的 closed 结论代表共享 Plugin Catalog/消费者基础已经成立，
   不代表 MiniApp Active Release 的正式 publication→shared Catalog→consumer
   生产链已经完成。先复用现有 canonical materializer/resolver、provenance、
   availability 和 exact lock，再决定最小的 MiniApp 接入切片。
3. MiniApp 的具体 Workspace、Knowledge、Connector 等资源仍在消费目标/会话中绑定；
   不把具体资源写回 AgentPreset、Revision 或 Snapshot。
4. Active Release、Release identity、owner、epoch、Catalog publication 必须原子切换；
   失败保持旧 Active/Catalog，不接受半新半旧状态。
5. 旧 Extension/MiniApp 生产链一旦新主链可用就物理删除；不双读、双写、alias、
   fallback、隐藏入口或 metadata-only 成功。
6. 不因为单个 crate、UI 页面、Gate self-test 或 mock 通过就关闭整项台账。

四、第一轮实际工作建议

先做小范围只读勘察，再实现一个最小可验证闭环：

1. 检查 `crates/backend/nomifun-miniapp-platform/src/m1_application.rs`：
   当前 `MiniAppWorkshopDto.capabilities` 仍是空投影，MiniApp 自己的
   `miniapp_catalog_publications` 不能被当作共享 Catalog。
2. 检查 `crates/backend/nomifun-agent-platform/src/platform.rs`：
   `KernelCatalogProvider` 当前主要物化 Kernel Plugin Registry；确认如何在不
   复制 Catalog 的前提下接入 MiniApp publication。
3. 检查 `crates/backend/nomifun-agent-control-plane/src/catalog.rs`、
   `crates/backend/nomifun-agent-contracts/src/catalog.rs`、现有
   `CatalogProvider`/materializer/resolver 和 `plugin_platform` 的调用链。
4. 检查 `crates/backend/nomifun-app/src/services.rs`、相关 MiniApp router 和
   App composition，找出 Active Release 切换与 Catalog publication 的共同事务边界。
5. 再扫描旧生产依赖，重点包括：
   `crates/backend/nomifun-extension/**`、
   `crates/backend/nomifun-channel/src/routes.rs`、
   `crates/backend/nomifun-gateway/src/caps_mcp.rs`、
   `crates/backend/nomifun-gateway/src/deps.rs` 以及 App startup/composition。
   只删除已经确认迁出的生产路径；测试支持代码必须按 06/台账逐项判断。
6. 先补最小 contract/application/route 测试，再进入 UI 或 Windows Candidate；
   不要先写大规模抽象、通用状态机、第二套 registry 或未来 Marketplace。

五、当前已交付事实

1. AP-0～AP-7、一期 Windows C8 与 `SL-S0`～`SL-S4` 的既有闭环已由台账记录为
   closed；Nomi-core 是当前产品执行内核，Codex Sidecar 只是未来研究，不是本阶段
   前置。
2. AgentPreset 产品纠偏已完成：首页可选 AgentPreset，Agent 工作台可查看能力
   明细、三态、删除和“使用 Agent”预选；Preset 会话锁定 Snapshot 模型；普通 Nomi
   保留模型选择；Workspace/Knowledge 在消费目标/会话中选择。
3. Plugin N1 已交付机器合同、Artifact Store、Host/Runtime foundation、owner
   mutation、部分 application service 和共享 Catalog 基础；N1-2-03、N1-4-01、
   N1-U-01 等仍须以实时台账为准，不得凭名称提前关闭。
4. MiniApp M1 已交付：
   - 新 Product/Project/Ready/Active/Previous 数据根；
   - UI-only Build/Publish/Rollback/Surface/Host KV；
   - dedicated Service Host、Node process、MessageChannel 和 generation fence；
   - owner-scoped KV、Files、Private SQLite、authorizer、参数化 SQL、Migration ledger；
   - Enable/Disable/Trash/Restore/Permanent Delete 与 restart recovery；
   - Service Test transient namespace/receipt；
   - Share Bundle、source-less prebuilt Import-as-new；
   - Desktop Transfer UI；
   - disabled Whole-App Backup Export/Import-as-new。
5. `M1-2-01` 已在 2026-09-09 关闭。Whole-App Backup 已验证 Service Files、
   Private SQLite、Migration ledger、Release identity 重绑定、按新 identity 重算
   Catalog digest、Credential slot union、owner operation 互斥，以及 staging +
   quarantine 原子恢复。

六、当前台账重点

以 `PHASE-N1-M1-CLOSURE-TODO.zh.md` 的表格和最新追加记录为准。当前不能提前宣称
关闭的重点包括：

- `N1-2-03`：Plugin application-service 的完整 Candidate/Apply/Restore/fault/安装版闭环；
- `N1-4-01`：production registry/lock mutation、MiniApp profile 和 Chat Dev Source edit；
- `N1-4-02`、`N1-4-03`：依赖前两项的 Candidate、Share/prebuilt/CLI；
- `N1-X-02`：旧 Extension loader/registry/hub/permissions/settings/webui/agent/theme
  生产链物理删除；
- `N1-U-01`、`N1-V-01`：Plugin UI/Runtime 产品走查与 Windows Candidate；
- `M1-U-01`、`M1-V-01`：MiniApp Desktop 产品/accessibility 和 Windows Candidate；
- `RC-WIN-01`：最终 Windows source cohort、NSIS、installed-app、fault 和 release lock。

如果实现发现台账的状态与代码/提交不一致，先在台账追加明确的状态核对记录，再继续；
不要用一次性摘要覆盖台账。

七、验证与 Provider 规则

文档改动不需要全量构建。代码改动选择直接覆盖变更行为的最小测试。稳定基线可先运行：

    cargo test --locked -p nomifun-miniapp-platform --tests -- --test-threads=1
    cargo test --locked -p nomifun-db --test miniapp_m1_repository -- --test-threads=1
    bun run check:i18n
    bun run build:ui
    git diff --check

模型相关 Chat Dev/E2E 固定使用 StepFun Coding Plan 的 `step-3.7-flash`。真实 smoke
只能通过 Windows Credential Manager runner：

    powershell.exe -NoLogo -NoProfile -ExecutionPolicy Bypass -File .\scripts\validation\run-nomi-core-live-provider-from-windows-credential-manager.ps1

凭据只能从受控 Credential Manager 读取；如果目标不存在，由操作者在本机使用脚本的
`-Setup` 隐藏输入一次性配置。绝不在聊天、Prompt、源码、文档、fixture、日志、argv、
环境快照、提交或报告中写入或打印 API key。真实 smoke 只覆盖它实际执行的产品合同，
不能替代其他测试，也不能用 synthetic PASS 绕过故障。

八、并行与交付纪律

如果当前 coding agent 支持子任务，可按互斥写集并行：

- Catalog lane：只处理共享 Catalog publication/resolver/consumer 的最小闭环；
- Legacy cleanup lane：只做生产 Extension/MiniApp 依赖审计和物理删除；
- Windows/UI lane：只处理 MiniApp/Plugin Desktop 产品、accessibility、Candidate 脚本。

中央 contract、App composition、`Cargo.toml`/`Cargo.lock`、Gate、GLOBAL/PHASE 台账
必须由集成 Owner 串行合流。不得同时编辑同一文件，不得并发争用同一数据库、固定端口、
Cargo release 构建目录、NSIS 安装进程或同一 Desktop 进程树。

每个实际闭环单独提交，提交前必须：

1. `git status --short`；
2. 只按明确路径 `git add`，禁止 `git add .`；
3. `git diff --cached --check` 和 staged diff 审阅；
4. 运行对应最小验证；
5. 更新 `PHASE-N1-M1-CLOSURE-TODO.zh.md`，必要时更新 GLOBAL 的 checkpoint；
6. 使用普通 commit 并 `git push origin rf/agent-capability-platform-v2`；
7. 在交付报告中列出 changed paths、命令、通过/未运行项、阻塞原因和新 commit。

九、停止边界

当前机器先完成 Windows N1/M1 与 `RC-WIN-01`；在此之前不领取 macOS/Linux 开发或
原生验收任务，不运行手机模式。若遇到需要真实签名、macOS arm64 或 Linux Desktop
环境的事项，记录为 `external`，不要用本机模拟结果关闭。
```

## 当前基线与工作树

- 分支：`rf/agent-capability-platform-v2`
- HEAD：`51b0243f7587df22a4007ad63655b64d0f861b7a`
- `origin/rf/agent-capability-platform-v2` 与 HEAD 已对齐（ahead/behind：`0/0`）。
- 最近交付提交：
  `feat(miniapp): close whole-app backup transfer`
- 当前工作树没有 tracked 改动；本地 `.githooks/` 是未跟踪开发辅助目录，按仓库约定
  保留但不要提交、删除或纳入交接制品。
- 上一轮未完成的 MiniApp Catalog 泛化实验已全部恢复，不属于当前基线；不要从未提交
  文件、旧会话或 Git 历史恢复它。

## 已验证的基线检查

2026-09-09 在 Windows 主机对当前基线执行了以下最小检查，均通过：

```text
cargo test --locked -p nomifun-miniapp-platform --tests -- --test-threads=1
cargo test --locked -p nomifun-db --test miniapp_m1_repository -- --test-threads=1
bun run check:i18n
bun run build:ui
git diff --check
```

UI production build 存在既有的 chunk size 和 dynamic-import 警告，但构建成功；不要把
这些非阻断警告扩大为无关的性能重构任务。

## 交接完成判定

下一台机器开始编码前，应在自己的首个交付报告中确认：

1. 已从远端 `51b0243f7`（或其明确后继）快进同步；
2. 已阅读 05、06、GLOBAL、PHASE 和本文；
3. 已确认没有把 API key 写入任何仓库内容；
4. 已确认第一项工作是共享 Catalog 的 MiniApp consumer integration，而不是复制
   Agent Catalog 或恢复旧兼容层；
5. 已给出本轮互斥写集、最小验证和预计提交边界。

## 2026-09-09 当前主机继续实施 checkpoint

原始阶段性交接基线 `51b0243f7` 已由以下后继提交推进：

- `926ad5d31`：MiniApp Agent exact Snapshot projection、Compiler、同一 Nomi
  hosted Tool session、Release Store schema revalidation 和 Service Host dispatch；
- `f4d71a133`：同步 GLOBAL/PHASE/06 台账与设计注记；
- `b6c9c2e89`：修正 Guid 启动入口：普通 Nomi 才选择模型，AgentPreset/官方模板
  使用服务端冻结 Snapshot，不向 Session Create 提交客户端模型。

当前远端分支已包含上述提交；下一台机器必须先 `git pull --ff-only` 并阅读最新
台账，不得把原始 `51b0243f7` 当作最新 HEAD。`gate:agent-v2 --self-test`、
`gate:plugin-n1 -- --self-test`、Guid 定向测试和 UI production build 已通过；
全量 UI typecheck 仍有既有基线错误。旧 Extension/旧 MiniApp 当前不存在可直接
安全删除的生产代码集合；Channel、Gateway、App、UI 和历史 schema 仍需先完成
前置迁移，N1-X-02 保持 blocked。`.githooks/` 继续保持未跟踪且不纳入提交。
