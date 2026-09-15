# macOS 多 Engine 接手 Prompt

> 2026-09-15 当前接续结果见 [MACOS-REVALIDATION-2026-09-15.zh.md](MACOS-REVALIDATION-2026-09-15.zh.md)。
> 下文 Windows CONFLICT、旧门禁失败和授权描述保留为历史阶段上下文；本轮用户要求后续修改不得推送，实际仅本地提交。

本文提供可直接交给 macOS 开发代理的任务说明。源码推送不等于产品验收完成。
交接基线包含 `c619d8a9a`（官方模板引擎选择）、`739b5e431`（完整引擎实现与
Windows 开发基线），以及与远程 `16dabde80` 的合并提交 `4a774ee59`；请以重构分支最新提交为准。
历史切片文档中的“未提交/不 push”描述对应当时状态；用户已在本轮授权提交与推送。

---

你正在 macOS 上接手 NomiFun 多 Engine 重构，请直接实施必要修复、验证与打包，
目标是形成可复现的 macOS 开发/交付基线，不扩大为新一轮架构重写。

## 一、分支与工作规则

- 所有实现统一在 `rf/agent-capability-platform-v2`，不切换到主分支开发。
- 先读仓库及子目录 AGENTS.md，检查工作区，再 fetch；干净且可快进时使用
  `git pull --ff-only origin rf/agent-capability-platform-v2`。有本地改动或分叉时先保护，
  不 reset、不强推、不覆盖他人工作。
- 确认上述两个基线提交已在 HEAD 历史中，不能只拿到官方模板的 UI 提交。
- 多代理可以并发，但按文件/模块划分互不重叠的写入范围；Cargo 构建串行，避免争锁。
- 只做 macOS 必需适配及双引擎现有缺陷修复；保留 Windows 行为，Linux 继续记录 TODO。
- 使用隔离数据目录、临时工作区与本地测试制品，不覆盖真实用户数据库和已有安装。
  不重写迁移 checksum，不自动删除不兼容数据库；签名、发布、push 另按用户授权执行。

## 二、不可变的产品与架构边界

- Engine 决定任务执行循环、规划方式、上下文策略。Agent 决定选择的 Engine、模型及能力配置。
- 官方预置 Nomi 通用 Engine 与 Coding Engine；不能把 Coding 降格为 Nomi 的提示词/profile。
- 引擎只允许源码二次开发、随应用编译打包，并在组装阶段注册冻结；禁止安装后挂载、
  上传执行器、动态库热加载，也不要恢复已退役的外部 Codex Wrapper。
- 引擎选择入口在 Agent 工作台，不在全局模型页或会话输入框。
  个人 Agent 在“基本设置 → 运行时引擎”保存；官方模板可以先选引擎，再“保存为我的 Agent”。
- 既有 Session 保持精确 build/digest/profile 绑定；缺失版本明确报错，不静默换引擎。
  Fork 保留精确绑定，不重新解析 stable；不要借平台适配自动改写旧会话。
- 平台继续拥有 Session、权限、模型凭据、工具与进程执行所有权、历史及副作用凭据。
  不增加平行 Session 数据库；清理完成后才发布终态，未知执行结果不能当成功或自动重放。
- Coding 就是第二种真实接入实现，不新增、不验证独立社区 Engine 示例作为验收前提。
- 本轮不做 SDK 大重构；仅记录现有接口分散、Nomi 专用路径等开发体验改进建议。

## 三、先阅读的资料与源码

本目录优先读取：

1. `STATUS.zh.md`、`TASK-MANIFEST.json`：总状态；历史切片不是最终通过证明。
2. `WINDOWS-DELIVERY-HANDOFF-2026-09-14.zh.md`：Windows 工程检查及迁移边界。
3. `WINDOWS-PACKAGE-LIVE-2026-09-14.zh.md`：安装包和真实模型的失败/未验证项。
4. `RUNTIME-EXTENSIONS.zh.md`、`ENGINE-COMPOSITION-2026-09-14.zh.md`：编译期接入。
5. `ENGINE-PACKAGING-CUTOVER-2026-09-14.zh.md`：macOS 旧执行器移除与 release-lock v2。

核心入口：

- `crates/backend/nomifun-ai-agent/src/engine_sdk.rs`、`runtime_catalog.rs`、`runtime_admission.rs`
- `crates/backend/nomifun-engine-core/`、`crates/backend/nomifun-coding-engine/`
- `crates/backend/nomifun-app/src/router/runtime_engines.rs`、`engine_session_host.rs`、
  `coding_runtime_host.rs`、`nomi_core_session.rs`
- `scripts/run-dev.mjs`、`scripts/desktop-build-mac.sh`、`scripts/validation/check-macos-arm64-native.mjs`

## 四、已知事实：不要误报通过

- Windows 曾通过后端/桌面编译、Coding/Core/迁移定向回归及 UI 类型检查。
  合并后的结果需另记，不直接沿用旧提交的通过结论。
- Windows 0.7.6 未签名 NSIS 包已构建并通过完整性检查；不是安装/升级/卸载全生命周期验收。
  该包早于本次官方模板入口和上游合并，不能代表最新源码制品。
- StepFun 正确模型为 `step-3.7-flash`，endpoint 为 `https://api.stepfun.com/step_plan/v1`。
  直接 API 探测返回过 200；双 Engine 主链仍在 `engine.nomi.create` / `engine.coding.create`
  阶段报 `CONFLICT`，尚未定位根因，也没有首轮工具成功证据。优先检查合并后能否复现，
  安全地补充内部诊断并修真正原因，不能放宽权限/资源准入让测试表面通过。
- Windows 开发启动已通过使用新的隔离 dev 数据根修复；旧 MiniApp 数据库不兼容当前
  Plugin clean-start 迁移。这个默认目录调整仅适用 Windows，macOS 需要核对实际行为，
  不直接把 Windows 路径或迁移账本补丁搬过来。
- macOS 构建脚本已移除外部 Runtime 导入，尚无本次 macOS 原生构建、UI、进程回收验收。
- 推送前综合检查未全绿：`check:agent-vocabulary` 检出 111 处旧术语引用，包含历史归档、
  必须保持兼容的 `miniapp` 持久化编码，以及活动代码和过时的 Coding 能力提示。
  不要全局字符串替换或关闭门禁；区分历史/存储契约与可更名的实现、文案，按实际支持能力修正。
- `.git/` 下日志和 `dist/` 制品不随源码推送；macOS 必须生成自己的证据，不假定文件存在。

## 五、实施顺序

1. 记录 macOS、CPU、Xcode/CLT、Rust、Bun 版本，安装锁定依赖，先跑原生架构。
   Apple Silicon 优先 arm64；不把 Rosetta 运行当原生证明。
2. 用隔离 `NOMIFUN_DATA_DIR` 启动 `bun run dev`。检查窗口可交互、前后端连接、
   官方两个引擎可发现，官方模板与个人 Agent 都能保存引擎配置并创建新会话。
3. 针对性检查：UI typecheck/桌面边界，Engine Core/Coding 单测，应用/桌面编译，
   迁移与受影响路径回归。使用仓库现有脚本，不恢复退休 C1-C9/外部 Wrapper 门禁。
4. 重点修复 macOS 路径、大小写/符号链接边界、文件权限、shell/env、
   子进程树取消/超时/退出回收、流取消及重启恢复；保留 Windows 已有语义。
5. 处理双引擎创建 `CONFLICT`。真实模型测试使用既有
   `scripts/validation/run-nomi-core-live-provider-smoke.mjs --engine-smoke`，
   先检查脚本支持的参数及隔离/清理流程。
   只使用用户安全提供的 `NOMIFUN_LIVE_STEPFUN_API_KEY`，模型为 `step-3.7-flash`；
   不把 key 写进代码、prompt、报告、命令参数或日志，不自动换模型/endpoint。
   没有凭据就报告真实测试阻塞，继续不依赖凭据的工作。
6. 双引擎覆盖文件创建/读取/修改、只读命令、多轮续接、取消与正常关闭；按风险
   补充上下文压缩、Fork、精确绑定与恢复。测试未执行或失败须分别列出。
7. 打包先用 `bun run build:mac arm`（Intel 机器选 intel）。注意无架构参数默认 Universal；
   不必要时不要同时构建所有架构。检查 app/DMG 结构、原生架构、资源、执行权限和
   无旧执行器残留，在隔离环境启动包内程序并正常退出。
   签名、公证仅在凭据和授权齐备时执行，否则明确标为未执行；不发布 Release/更新。
8. 更新本目录 macOS 交接记录：源码 commit、命令、实际结果、制品 SHA-256、
   修复文件、剩余阻塞及 Windows 需回归的公共代码。不要把打包成功写成完整交付。

完成条件：macOS 原生开发启动和包内启动可复现，双引擎选择及执行链有真实证据，
取消/退出没有遗留受管进程；未通过项、签名状态与平台限制清楚记录。无法完成某项时
保留最小复现和明确阻塞，不发散到新引擎/新平台/新 Provider 开发。

---

## 本次合并后的 Windows 复核（2026-09-14）

- `cargo check --offline -p nomifun-desktop -p nomifun-app --lib --bins --test coding_runtime_production`：
  通过，耗时 6m42s；应用库仍有 28 项 warning，不等于运行测试或打包验收。
- `agent-v2-contract write` 后 `check`：通过，已按合并源码重生成摘要。
- Agent/UI 定向测试：83 通过、0 失败，包含官方模板引擎选择。
- dev runner、macOS 打包契约、release-lock、macOS 验证 helper 的脚本测试：
  24 通过、0 失败；均在 Windows 执行，不是 macOS 原生结果。
- TypeScript、桌面边界、i18n、theme/icons/dead CSS、Windows 安装契约、Creative Studio
  退役、进程边界通过。Browser 门禁已修复对退役宿主的要求，自测及扫描通过；
  automation-session 边界与 `help --check` 通过。
- `bun run check` 首次暴露 i18n 生成索引漂移，修复后继续暴露退役 Browser 根要求；
  两项均已修复并定向复核。最后的 Agent vocabulary 仍失败，未将综合检查记为通过。
- 本次没有重跑 Rust 完整测试、真实模型调用或安装包；历史结果的适用范围见前文。
- 未提交凭据、`.git/` 日志或本机制品；源码及待办一起交接。
