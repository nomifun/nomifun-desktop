# macOS 多 Engine 开发与交付验证

日期：2026-09-14。本文记录本轮 macOS 实际执行结果；Windows 旧结果不作为本机通过证明。

## 源码与范围

- 分支：`rf/agent-capability-platform-v2`。
- 初始工作区干净；`git fetch origin`、`git pull --ff-only origin rf/agent-capability-platform-v2` 成功。
- 同步基线：`2ff02951266b12008cf1f7ea63c3b27cff9a192a`，包含 `c619d8a9a`、`739b5e431`；初始本地/远端差异为 `0 0`。
- 本轮修复将在本地提交后重建，以满足 release-lock 的干净源码认证；未 push、未发布 Release 或更新。初始基线不代表最终制品源码，最终以制品 release-lock 的 source_commit 为准。
- 仅官方 Nomi/Coding 源码编译注册；未恢复 Wrapper、未运行独立社区 Engine 示例、未更改历史 SQL 或持久化编码。
- 首轮按模块分工，Cargo 串行；用户要求减少并发后，由主代理串行收尾，不再新增代理。

## 环境与隔离

- macOS **26.6.2 (25G83)**，CPU/执行架构 **arm64**。
- Xcode **26.5 (17F42)**；SDK：`/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk`。
- SDK **26.5**，Apple clang **21.0.0**；独立 CLT package **26.3.0.0.1.1771626560**，实际选择的 developer directory 为 Xcode 内目录。
- Rust **1.96.0**，host `aarch64-apple-darwin`；Cargo **1.96.0**；Bun **1.3.14**。
- `bun install --frozen-lockfile --ignore-scripts` 通过，无依赖/锁文件升级。
- 本机证据根：`.git/macos-engine-20260914/`，不随 Git 提交。开发数据、模型 fixture、包内启动使用隔离目录，不访问真实用户数据库。
- 真实模型严格使用 `stepfun-plan`、`https://api.stepfun.com/step_plan/v1`、`step-3.7-flash`。凭据从隐藏终端输入进入 runner 环境，Cargo 子环境剔除凭据；测试二进制仅经 stdin 接收，工具子进程不继承。未把 key 写入源码、命令 argv 或报告。
- 构建前保存了原 `dist/desktop/NomiFun_0.7.6_aarch64.dmg` 和 release-lock 的独立副本及 SHA-256，位置为证据根下 `prior-artifacts/`。已有安装未覆盖。

## 缺陷、修复与实际证据

### 双 Engine 首轮 CONFLICT

`engine.nomi.create` / `engine.coding.create` 是首轮创建文件任务，不是 Session 创建接口。
首轮本机真实测试复现两个 `CONFLICT`。同构 loopback mock 随后暴露具体原因：
`Agent Skills: requested Skill is not in the Agent's immutable selected Skill locks`。

`ConversationService` 在 canonical Agent 的不可变 Skill locks 之外，合入安装级自动 Skills。
修复创建、切换 Agent、更新 capability-selection 三条写入路径：canonical snapshot 只使用
已选择的 Skills；legacy 自动注入行为和 immutable Skill gate 保留，未放宽权限。
Nomi 构建标识前进为 `0.7.6-host61`，Coding 为 `0.7.6-host2-coding-loop92`；源码摘要包含修复。
旧会话不自动改绑或清洗，缺失 exact build 仍明确拒绝。

官方双引擎 loopback 回归已通过：三条写入路径、首轮模型响应、Fork 精确绑定、修改 Agent
后仅新会话选择新 Engine、父/子会话维持原绑定。另一个回归在确认 Provider 收到请求且
会话 running 后取消，再验证同会话续接及宿主关闭；两个 Engine 均通过。
这些是本地 mock 的真实平台路由测试，不能替代真实模型工具结果。

第二轮真实测试 Nomi 的 create/patch/exec/continue 四阶段全部通过；Coding 首轮工具失败。
进一步本地回归证实两项原因：

- 旧烟测要求“只调用写文件工具”，与 Coding 必须 update_plan / report_completion 的内部
  执行政策冲突。fixture 现允许内部控制工具，仍严格核对外部文件/命令工具参数、实际文件、
  输出和多轮历史；控制工具的错误不隐藏，并要求成功完成报告。没有关闭生产规划门禁。
- Coding 把完整操作身份直接用作工具幂等键，超过 Wave 2 owner 的 128 字节上限。
  改为对完整身份生成 SHA-256 key，不截断、不放宽 owner 上限、不迁移旧执行记录。

包含真实文件写入、规划/完成报告、Fork 和取消续接的本地官方 Engine fixture 最终
**13 通过、0 失败**。定位 INVALID_PAYLOAD 时仅对 loopback/dummy 凭据回归使用了临时
内部诊断；已移除诊断源码，之后才重新开始真实模型测试。

真实测试历史（所有运行均固定同一 Provider/模型，无 fallback）：

| 运行日志 | Nomi | Coding |
| --- | --- | --- |
| `live-engine-1.log` | 首轮 CONFLICT | 首轮 CONFLICT |
| `live-engine-2.log` | 四阶段通过 | create `TOOL_OR_TURN_FAILED`，未细分 |
| `live-engine-3.log` | 四阶段通过 | create `TOOL_OR_TURN_FAILED`，未细分 |
| `live-engine-4.log` | 四阶段通过 | create 阶段超时；只读观察已 finished，写入/完成报告/最终标记已出现，旧 fixture 对内部上下文工具的识别仍不完整 |
| `live-engine-5.log` | **四阶段通过** | **create 通过；patch `UNKNOWN_UPSTREAM_ERROR`；exec/continue 未执行** |

第四轮后修正 fixture：允许成功的引擎内部计划/历史检查，以及仅针对目标文件或工作区根的
instruction_scope 读取；普通文件读取、未知工具、运行中/失败工具不隐藏。增加正负回归。
已稳定 ready 但证据不完整时，在两秒跨查询宽限后返回具体错误，不再由外层 180 秒超时
遮盖诊断。第五轮由此取得 Coding 首轮实际工具成功证明，但第二轮错误尚未定位。
本地 mock 额外验证“完成工具/计划回合后同 Session 下一轮”通过，因此不能把真实错误
直接归因为通用恢复缺陷或 StepFun 服务；保留原错误和最小复现，不宣布 Coding 主链完成。

上述 `422/408` 是 harness 的错误报告状态，不能据此推断 StepFun 的 HTTP 状态。
每次 runner 在返回阶段失败前仍完成宿主关闭和凭据未持久化审计；未做真实 Provider
取消、上下文压缩或重启恢复验收，mock 和进程模块结果不能替代这些项目。

### Fork 消息落库

新增完整历史 Fork 回归发现 `NOMI_CORE_FORK_STORAGE_FAILED`。原复制消息生成 UUIDv5，
而既有 `messages` 表要求 UUIDv7 格式。修复新复制消息的确定性 ID 格式，保留原存储约束，
不修改 SQL 或历史消息。修复后官方双引擎 Fork 回归通过。

### 合并后迁移认证

初次 runtime 迁移回归为 **3 通过、2 失败**。Windows 实施基线仍包含 027，
上游 `2eb642035` 删除 027 后由 `4a774ee59` 合入；原认证仍错误假定编号连续。

runtime 迁移认证改为固定集合 `001..026、028..094 + displaced 095`，Agent preset 认证
改为 `001..026、028..087 + displaced 088`。两者先核对 embedded 版本集合，再逐行
核对 version、success、exact checksum，不认证未来任意增删的历史，也不修改 SQL/checksum。
实际携带额外 027 的历史账本仍拒绝，不自动删除该行。

新增负向回归还发现 Agent recognizer 在缺行时返回 false，导致普通 SQLx 先补 028、
随后因 088 冲突失败。修复为识别 displaced/unknown 088 后立即严格拒绝不完整前缀，
阻止失败前的补迁移。原失败日志保留；此前测试自动删除的临时数据库不宣称仍存在。
新的失败回归会保留数据目录并仅输出变化版本集合。

### macOS 退出和 WebView 数据隔离

初次 `bun run dev` 可启动 arm64 后端与 renderer；向 Tauri CLI 发 SIGINT 后观察到桌面进程
遗留并被 reparent 到 PID 1。该次以 SIGTERM 清理，不计正常退出通过。

macOS runner 现在用本轮独占 Unix socket 表达生命周期，先请求桌面经过既有 ExitCoordinator
清理，再结束 CLI。桌面 SIGINT/SIGTERM 同样进入该正常退出路径，不给受管工具抢先发组信号。
修复后向 runner 发 SIGINT，观察到 channel/terminal 清理日志，runner、CLI、Vite、app 均退出；
无强杀。相关证据为 `dev-fixed.log`、`dev-exit.json`。

另发现全新后端数据根仍读到旧 renderer 工作区：WKWebView 默认 store 未隔离。
对显式自定义 `NOMIFUN_DATA_DIR` 增加按 canonical 根生成的独立持久 store，main/companion
使用同一标识。默认根（含重启继承和符号链接别名）继续使用原 profile，不迁移、清空它。
自定义隔离 store 需要 macOS 14+；旧系统明确报错，禁止静默回落默认 profile。

最终以 `NOMIFUN_DATA_DIR=<证据根>/dev-isolated-final bun scripts/run-dev.mjs --no-watch`
验证最新源码（前面已实际执行两次 `bun run dev`；此处直接使用同一 runner 避免重复缓存清理）。
health 200，renderer 请求中不再出现旧工作区。随后直接向 Tauri CLI 发 SIGINT，四个已记录
PID 均退出，日志含 channel/terminal 清理；runner 的 130 是 SIGINT 常规退出码，不写成 0。
见 `dev-isolation.json`、`dev-cli-exit.json`、`dev-isolated-final.log`。

开发二进制没有 `.app` 身份，当前 CUA 无法按名称、bundle ID 或可执行路径识别；
因此开发窗口人工交互仍未验证。后端请求证明 renderer 运行，不替代窗口操作证据。

## 工程检查

| 检查 | 本机结果 |
| --- | --- |
| 原生开发编译/启动 | 通过；初次有退出缺陷，修复后 runner 退出已通过 |
| Agent/UI 定向回归 | 最终 85 通过、0 失败；有 React ref/act/模拟高度警告 |
| Core/Coding 单测 | 最终 14 + 37 通过、0 失败，含工具幂等键边界回归 |
| 进程模块定向 + recovery | 65 + 6 通过、0 失败；含复杂路径、symlink、shell、取消、PTY、父进程退出、PID identity |
| 初始四组脚本测试 | 24 通过、0 失败 |
| 修改后 dev runner 测试 | 11 通过、0 失败 |
| 官方 Engine fixture / 证据回归 | 13 通过、0 失败，包含工具写入、规划、Fork、in-flight 模型取消续接；无社区 Engine |
| 修复后迁移 | 7 项模块 + 5 项集成，全部通过；原失败独立保留 |
| macOS 桌面定向测试 | 5 通过、0 失败，含 WebView store、窗口 reopen 和 `/var` 别名 |
| legacy Skill 合并回归 | 4 通过、0 失败 |
| `agent-v2-contract check` | 退出码 0；无需生成文件改写 |
| 综合 `bun run check` | **失败**：旧术语 109 处，较初始 111 处纠正两条 Coding 提示；未关闭门禁或替换历史编码 |
| TypeScript / 桌面边界 / i18n / theme / icons / dead CSS / Windows 安装契约 / retirement / process / browser / automation-session | 综合检查中到旧术语门禁前均通过 |
| `bun run help --check` | 单独执行通过（综合链因前项失败未走到此步） |
| macOS arm 工具预检 / live runner self-test | 通过；工具预检不代表实际制品 |

## 本轮修复文件与源码对应

- `apps/desktop/src/main.rs`、`scripts/run-dev.mjs` 及脚本测试：macOS 生命周期、显式数据根 WebView 隔离。
- `crates/backend/nomifun-conversation/src/service.rs`：canonical Skill 选择边界。
- `crates/backend/nomifun-app/src/router/nomi_core_session.rs`：新 Fork 消息 ID 合同。
- `crates/backend/nomifun-app/src/router/runtime_engines.rs`、`coding_runtime_host.rs`：官方构建身份前进。
- `crates/backend/nomifun-coding-engine/src/tool.rs`：有界且保留完整身份的幂等键。
- `crates/backend/nomifun-db/src/database/displaced_*_migration*` 及直接测试：合并后固定版本集合和失败前拒绝。
- `crates/shared/nomi-process-runtime/tests/{request_contract,process_contract}.rs`：macOS 路径和 shell 回归。
- `ui/.../engineRevision.integration.test.tsx`、中英文 `agentSettings.json`：官方模板保存覆盖和准确能力提示。
- `crates/backend/nomifun-app/tests/nomi_core_live_provider_smoke.rs`：官方 Engine 平台/真实模型证据及安全诊断。

构建前已冻结上述 19 个非文档变更文件的完整覆盖包 `source-overlay.tar.gz`，SHA-256：

```text
9393b60e9c5775806b3a202c1c26a10eb0cf7854909913a961e8a5c812e48579
```

覆盖包、逐文件 `source-manifest.json` 和 tracked diff `source.patch` 均位于本机证据根。
制品的完整源码描述为基线 commit **加该覆盖包**；release-lock 中的基线 commit 不能独自
代表未提交工作区。文档更新不包含在代码覆盖包中，避免报告摘要自引用。

## 首次打包检查失败与收敛

`bun run build:mac arm` 已完成 arm64 Release 编译（12m51s）并生成 app/DMG，
随后 release-lock 创建以退出码 3 拒绝：`cannot attest source_commit from a dirty tracked worktree`。
没有跳过或降低门禁。将本轮代码和当前检查记录做本地提交，再从干净源码重建；
最终制品和认证以重建结果为准。首次日志为 `package-build.log`。

## 待完成的本轮记录

以下仍由主代理执行，完成后更新实际结果：WebView 隔离启动、
真实模型八阶段、原生 app/DMG、包内窗口交互与退出、制品 SHA-256、最终源码补丁摘要。
未完成的项目不会据编译成功推定通过。

Windows 需回归共享 Conversation Skills、Fork、迁移认证；Linux 运行验收继续 TODO。
Developer ID 签名、公证、发布、更新均未执行。
