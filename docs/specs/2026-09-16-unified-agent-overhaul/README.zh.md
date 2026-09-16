# 统一 Agent 架构重构：长程实施总计划

> 计划代号：UARC（Unified Agent Runtime & Capability）
> 日期：2026-09-16
> 当前主机：Windows
> 目标平台：Windows Desktop、macOS Desktop、Desktop WebUI；Linux 后续单独验收
> 状态：计划已建立，生产实现尚未开始
> 分支基线：`rf/agent-capability-platform-v2`

## 1. 目标与设计来源

本计划统一执行当前会话确认的全部重构：

- 单一官方 Nomi Runtime；
- Coding 与长程任务能力沉淀进统一 Runtime；
- 136 个碎片化 Capability 重组为领域 Module + Action 授权；
- AgentPreset、Skill、MCP、Resource、Provider 边界重构；
- 全新的 Agent Session/Event/Effect Store，Agent 数据 clean cut；
- Requirements/AutoWork/AgentExecution 收敛；
- Agent 路径删除独立 IDMM；
- Browser 从“会话浏览器”纠正为任意 Agent 可加载的平台能力；
- 删除旧 Nomi/Coding 双 Runtime、旧 Session 日志和兼容代码；
- Windows 与 macOS 原生实现及发布验证。

设计源：

1. [统一 Nomi Agent Runtime 与能力组合架构](../2026-09-16-unified-nomi-runtime-convergence.zh.md)
2. [Agent 能力模型全面检查与重构](../2026-09-16-agent-capability-product-redesign.zh.md)
3. [Agent Session 与执行日志存储重构](../2026-09-16-agent-session-storage-redesign.zh.md)
4. [Coding Preset 与 Runtime 兼容性检查](../../reviews/2026-09-16-coding-agent-runtime-compatibility.zh.md)
5. [Browser Windows/macOS 架构决策](../../continuity/2026-09-16-browser-platform-architecture-decision.zh.md)
6. [macOS CEF 实施检查点](../../continuity/2026-09-16-browser-cef-macos-implementation.zh.md)
7. [macOS CEF 接续 TODO](../../continuity/2026-09-16-browser-cef-macos-next-session.zh.md)

本目录负责施工，不重新讨论已经确认的产品决定。任务真相来自：

- `TASK-MANIFEST.json`：任务、依赖、写集、平台和门禁；
- `STATUS.zh.md`：唯一人工状态台账；
- `CONCURRENCY-MERGE-VALIDATION.zh.md`：并发、合并与测试协议；
- `MACOS-HANDOFF.zh.md`：macOS 实施与证据要求。

## 2. 完成边界

只有以下全部成立，UARC 才完成：

1. 产品只运行一个 `nomifun.nomi` Runtime factory。
2. 简单回合走最小路径，复杂/Coding 回合按事件启用长程机制。
3. AgentPreset 使用 Module + Action grants，不含 Runtime 选择和旧 Capability ID。
4. 六类官方 Agent、客服和自定义 Agent 从新 schema 创建。
5. 通用 Agent 默认启用 Skill/MCP/Schedule/Browser/Computer 模块。
6. 新 Agent Store 是唯一 Session/Turn/Event/Effect/Message 事实链。
7. 旧 Agent Session、Preset、Execution、Nomi transcript 不迁移、不读取。
8. Requirements、AutoWork、AgentExecution 共用新 Session receipt；Agent 路径无独立 IDMM。
9. Browser 没有专属 Session 入口，任意获授权 Agent 可使用统一 Browser Module。
10. 旧 Runtime、旧 projection、旧 store、旧 UI 和兼容路径物理删除。
11. Windows 全量门禁与原生验收通过。
12. macOS 补充实现、全量门禁、原生验收和打包检查通过。
13. macOS 合并后再次通过 Windows 回归。

Windows 完成不能把计划标成跨平台完成；macOS 项必须有 macOS 主机证据。

## 2.1 全局工程要求

### 删除是正式交付物

本轮不是在旧实现旁边追加一条新链。每个任务必须同时声明：

- `write_set`：新增或修改什么；
- `delete_set`：替代完成后删除什么；
- `retained_set`：哪些底层实现有继续复用的明确理由；
- `reachability_check`：怎样证明旧入口在生产不可达。

允许并鼓励删除已经被替代、不可达、重复或只服务历史模型的代码，包括：

- 生产模块和 DTO；
- Feature flag、环境变量和配置字段；
- 路由、命令、UI 入口和重定向；
- DB 表、migration 兼容器和私有文件格式；
- 测试 fixture、mock、脚本和生成规则；
- 过期文档、教程和产品文案；
- 注释掉的旧实现、deprecated wrapper 和无期限 alias。

禁止用“以后可能需要”为理由保留没有当前 owner、调用方和测试的代码。若底层实现仍复用，必须缩到
新边界内并重命名，不能保留旧产品身份。删除后运行 dead-code、生产 reachability 和反向搜索；
“新链可用但旧链仍在”不算完成。

### UI 改动必须作为产品设计交付

涉及用户界面的任务在写代码前必须提供最小交互设计，至少覆盖：

- 用户目标和主流程；
- 信息层级、入口位置和与现有页面的关系；
- default、loading、empty、disabled、error、partial、success 状态；
- 首次使用、无资源绑定、权限不可用和恢复路径；
- 键盘、焦点、屏幕阅读语义和可点击区域；
- 中英文文案，避免 Runtime、binding、canonical ID 等实现术语；
- 880×600 最小桌面窗口以及宽桌面布局；
- Windows/macOS 原生控件、权限和窗口行为差异。

UI 验收要求：

- 使用现有主题、间距、字体、图标和交互模式，视觉上像同一个产品；
- 不用临时按钮、裸 JSON、技术错误正文或仅开发者能理解的选择器完成交付；
- 不因能力丰富而把 20 多个模块平铺成难以理解的清单；使用分类、搜索、推荐组合和渐进展开；
- 关键流程有定向交互测试；布局改动运行 desktop UI boundary；
- Windows 进行真实桌面视觉检查，macOS 在对应任务中补充原生视觉、焦点和缩放验证；
- 一个功能若后端完成而 UI 仍是占位、难用或不一致，任务状态只能是 `implemented_unverified`。

## 3. 并发模型

### 3.1 固定并发上限

任何时刻最多 4 个活动席位：

| 席位 | 数量 | 职责 |
| --- | ---: | --- |
| Integration | 1 | 共享合同、基线、组合入口、合并队列、统一测试和状态台账 |
| Feature | 最多 3 | 互斥写集内的领域实现与定向测试 |

macOS Worker 占用一个 Feature 席位。macOS 启动后，Windows 同时最多保留两个 Feature Worker。

### 3.2 为什么不是更多并发

本次重构共享面很大：Contracts、Kernel、Store、Runtime、App composition、Catalog、Preset 和 UI DTO
互相依赖。超过三个功能任务会迅速增加：

- 同文件冲突；
- 生成物漂移；
- Cargo/链接器争用；
- 失败归因困难；
- 重复全量测试；
- Worker 使用不同合同版本。

先冻结共享切面，再并行领域消费者，实际吞吐高于无限开工。

## 4. 所有权与互斥写集

### 4.1 Integration 独占

下列文件/目录只能由 Integration 席位修改：

```text
Cargo.toml
Cargo.lock
package.json
ui/package.json
crates/backend/nomifun-agent-contracts/**
crates/backend/nomifun-api-types/**
crates/backend/nomifun-agent-kernel/**
crates/backend/nomifun-db/migrations/**
crates/backend/nomifun-agent-contracts/schema/**
crates/backend/nomifun-agent-contracts/contracts/generated/**
crates/backend/nomifun-app/src/services.rs
crates/backend/nomifun-app/src/router/{mod.rs,routes.rs,state.rs}
ui/src/renderer/components/layout/Router.tsx
ui/src/renderer/services/i18n/i18n-keys.d.ts
docs/specs/2026-09-16-unified-agent-overhaul/**
```

Feature Worker 若需要这些文件变更，只提交一条明确的 integration request：目标、必要字段、调用点和
测试，不直接修改。

### 4.2 领域写集

任务清单为每项任务声明 `write_set`。规则：

- 两个活动 Feature 任务的写集不得重叠；
- 不允许使用模糊的 `crates/backend/**` 写集；
- 测试文件与生产文件属于同一写集；
- 一个文件只允许一个活动 owner；
- 发现漏项时先由 Integration 更新写集，再继续编辑；
- Worker 不做 repo-wide rename、format、codegen 或依赖升级。
- 涉及 UI 的任务必须在 manifest 中声明 `ui_surface` 和 `interaction_spec`；未声明不得修改 renderer。

### 4.3 生成物

所有生成物由 Integration 在合并后统一生成：

- Contract envelopes/schema；
- i18n key types；
- Cargo.lock；
- review inventory；
- release lock；
- UI dist/build manifest。

Worker 只修改源文件。这样不会出现三个任务分别刷新生成物造成的合并风暴。

## 5. 实施波次

### Wave 0：基线与工作树收口（串行）

- `UARC-000`：冻结当前源、已有未提交改动和设计文档；
- `UARC-001`：建立任务清单、状态台账、测试基线和旧代码 reachability inventory。

出口：一个可复现的 Integration 基线；未归属的改动为零。

### Wave 1：共享合同与新事实模型（Integration 串行）

- `UARC-010`：Capability Module/Action/Resource 合同；
- `UARC-011`：新 Agent Store baseline；
- `UARC-012`：单一 AgentSession/Turn/Event/Effect owner；
- `UARC-013`：单一 Nomi Runtime Driver/host ports；
- `UARC-014`：AgentPreset vNext、Compiler 和通用 Capability projection。

本波禁止领域任务并行修改合同。每一项通过后形成 barrier commit。

### Wave 2：第一批并行实现（最多 3 Feature）

- `UARC-020`：Runtime 自适应执行、长程 Coding、恢复；
- `UARC-021`：Workspace Files/VCS/Process/Artifact modules；
- `UARC-022`：Skill/MCP/Plugin/Connector modules。

三项仅消费 Wave 1 合同，不互相修改代码。Integration 逐项合并，波末统一测试。

### Wave 3：第二批领域并行（最多 3 Feature）

- `UARC-030`：Web/Knowledge/Project Memory/Companion Memory；
- `UARC-031`：Channel/Companion/Customer Service；
- `UARC-032`：Creation/Workshop/Office/Attachment/Model roles。

### Wave 4：自动化、设备与 Browser（最多 3 Feature）

- `UARC-033`：Requirements/AutoWork/AgentExecution/IDMM 收敛；
- `UARC-034`：Schedule/Notification/Remote/SSH；
- `UARC-040`：Browser 产品模型与共享 Browser Resource/Provider 合同。

Browser Windows 原生和 macOS 原生都依赖 `UARC-040`，不能提前各自发明接口。

### Wave 5：平台与产品表面（Windows 最多 3 Feature；macOS 可占一个）

- `UARC-041`：Windows Browser resource/UI 改造；
- `UARC-042`：Computer/Robot 与 Windows native；
- `UARC-050`：官方 Preset、Agent Workbench、Module/Action UI；
- macOS 主机可并行开始 `UARC-061` / `UARC-062`，但总 Feature 席位仍不超过 3。

### Wave 6：切换与删除（Integration 串行）

- `UARC-051`：新 Agent Store/API/UI 切换；
- `UARC-052`：删除旧 Nomi/Coding Runtime 与多 Runtime 基础设施；
- `UARC-053`：删除旧 Capability ID/projection/Store/IDMM/Browser 入口；
- `UARC-054`：压缩新 baseline、移除历史 Agent migrations 和 fixtures。

本波不与领域开发并发，避免删除与功能修改互相覆盖。

### Wave 7：Windows 集成验收

- `UARC-060`：Windows 全量编译、测试、原生功能和安装包候选；
- 固定供 macOS 使用的 shared-source barrier commit。

### Wave 8：macOS 补充实现与验收

- Wave 5 未闭合的 `UARC-061/062` 必须先完成；
- `UARC-063`：在 Windows Wave 7 barrier 上重验 macOS 全量 Agent、原生 UI、DMG、签名结构和
  退出清理，闭合最终证据。

### Wave 9：回合并与最终门禁

- `UARC-064`：macOS 变更合入后 Windows 定向与全量回归；
- `UARC-070`：跨平台完成审计、文档收口和候选发布门禁。

## 6. Merge Queue

1. 每个 Feature 任务从当前 wave barrier commit 创建独立 worktree/branch。
2. 任务只改 manifest 声明的写集，形成一个主提交；必要的修复最多追加一个提交。
3. Worker 交付 diff 摘要、定向测试、未运行项和平台状态。
4. Integration 先检查写集，再检查行为和测试，最后按依赖顺序合并。
5. 每次只合并一个任务；合并期间暂停新的 shared-contract 修改。
6. 合并后只跑该任务的 integration checks；波末统一跑 wave gate。
7. Wave gate 通过后创建新 barrier；下一波所有任务从该 barrier 开始。
8. 发现共享合同错误时，暂停所有依赖任务，由 Integration 单独修复并发布新 barrier。

禁止：

- 多个 Worker 同时解决同一冲突；
- Worker 自行修改 Integration 分支；
- 为减少冲突复制第二份 DTO/schema；
- 在活动波中做跨仓格式化或批量 rename；
- 先合并失败任务再让其他任务适配；
- force-push 或重写已共享的 barrier。

## 7. 测试预算

### Worker Gate

每个 Worker 只运行：

- 自己 crate/package 的单元测试；
- 自己新增的集成测试；
- 必要的单 crate `cargo check/test -p`；
- 必要的单文件/目录 Bun tests；
- `git diff --check`。
- `delete_set` 反向搜索和新增 dead-code 检查；
- UI 任务的定向交互/结构测试和状态覆盖。

### Per-merge Gate

Integration 运行：

- 受影响 crate 编译/测试；
- Contract/DTO 定向测试；
- 相关 boundary script；
- UI 变更时 typecheck + 定向 UI tests。
- UI 变更时由 Integration 检查交互设计、文案、视觉一致性和所有状态，不只检查编译。

### Wave Gate

每个波只运行一次：

- `bun run typecheck`；
- 相关 Rust crate 组合测试；
- `bun run check:i18n`（涉及文案时）；
- `bun run check:desktop-ui-boundary`（涉及 renderer 时）；
- 受影响的 architecture boundary scripts；
- 波级端到端 fixture。
- 本波旧入口/旧模块 reachability 清零证明；
- UI 波次的 Windows 桌面视觉检查记录。

### Milestone/RC Gate

只在 Wave 1、6、7、8、9 结束运行：

- `bun run check`；
- `bun test --cwd ui`；
- `bun run test:core` 或范围等价的 workspace Rust gate；
- `bun run test:desktop`（桌面集成点）；
- 原生 smoke/安装包检查。

同一台机器：

- 同时最多一个 Cargo 编译/测试进程；
- 同时最多一个全量 Bun test/check；
- 不让多个 worktree 共享正在写入的构建目录；
- 长测试由 Integration 获取 test lease 后运行；
- 已通过且源码未变化的全量门禁不重复执行。

本仓库禁止 GitHub Actions。并发编排、门禁和平台证据全部使用本地脚本、worktree、状态台账和对应
操作系统的人工/原生验证，不新增 `.github/workflows` 工作流。

## 8. 跨平台规则

任务平台状态只能是：

```text
not_applicable
pending
implemented_unverified
verified
blocked
```

每项任务同时记录 Windows 和 macOS 状态。

- 纯合同/数据模型可以在 Windows 完成 shared verification；
- 带 `cfg(target_os)`、原生窗口、文件选择、权限、PTY、进程清理或打包的任务必须标记
  `macos: pending`，直到 Mac 真机验证；
- Windows 的 WebView2、Win32、NSIS 证据不能替代 macOS CEF child NSView、TCC、DMG/codesign；
- macOS 修复合并后必须回到 Windows 跑受影响回归；
- Linux 不在本轮完成定义中，但 shared code 不得硬编码 Windows/macOS 二选一。

具体 macOS 矩阵见 `MACOS-HANDOFF.zh.md`。

## 9. 状态与长程续接

`STATUS.zh.md` 只由 Integration 更新，时机限定为：

- 任务开始/释放；
- 合并完成；
- Gate 完成；
- blocker 变化；
- 平台证据加入。

Worker 不编辑状态文档，避免状态合并冲突。每条状态必须含：

- task ID；
- source/barrier commit；
- owner/write set；
- 实际变更；
- 测试命令和结果；
- Windows/macOS 状态；
- remaining work/blocker。

长程任务从 `STATUS.zh.md` 的“Next ready tasks”继续，不从聊天摘要或未提交临时笔记推断。

## 10. 当前下一步

1. `UARC-000`：整理当前 dirty worktree，将已授权的设置页与本轮设计文档归入明确基线。
2. `UARC-001`：生成生产 reachability、测试时长和平台缺口 inventory。
3. 完成 Wave 0 后，Integration 串行开始 `UARC-010`。

当前不得直接启动多个领域 Worker；共享合同和数据模型尚未冻结，提前并行只会制造返工。
