# macOS 多 Engine 当前 HEAD 复核（2026-09-15）

本记录接续 `MACOS-CONTINUATION-2026-09-15.zh.md`，保留先前记录的适用源码范围。
当前任务全部位于 `rf/agent-capability-platform-v2`，没有 push、Release 或更新发布。

## 源码与环境

- 开始时工作区干净，fetch 后本地与远程均为 `70c28b5de`，ahead/behind 为 0/0；
  `git pull --ff-only origin rf/agent-capability-platform-v2` 返回 Already up to date。
- `2ff02951266b12008cf1f7ea63c3b27cff9a192a`、`c619d8a9a`、`739b5e431` 均为 HEAD 祖先。
- 当前验收修复提交 `6125f1a64`：仅三个测试文件，不改生产执行逻辑、权限或迁移编码。
  本地提交用于让打包锁对应干净源码，没有推送。
- macOS 26.6.2 (25G83)，arm64，`sysctl.proc_translated=0`；Xcode 26.5 (17F42)、SDK 26.5，
  Rust 1.96.0、Cargo 1.96.0、Bun 1.3.14。
- `bun install --frozen-lockfile --ignore-scripts` 通过，无依赖/锁文件变更。
- 本次数据和日志根 `.git/macos-engine-70c28b5de/`；UI 分工日志在 `.git/macos-engine-current-ui/`。
  真实用户库和已安装应用未使用。旧 DMG/release-lock 已备份到本次证据根的 `previous-2ede9aaf7.*`。

## 实际修复及失败记录

1. Core/Coding 单测初次编译失败 `E0063`：`AgentPresetRevisionPayload` fixture 未提供新引入的
   `context_order` / `middleware_order`。在两个现有 Kernel 测试中补空顺序，不改变生产授权。
2. UI 引擎用例初次 27 通过 / 3 失败，单文件重跑复现。controller 已改用 `/api/agent-catalog`，
   fixture 仍模拟旧分散接口；对齐两处响应并新增加载无错误断言后 30/30 通过。
3. 非 Cargo 脚本初次误用 `node --test`，因 `bun:test` 无法导入失败；使用 `bun test` 后通过。
4. 原生验证的辅助脚本直接请求桌面 API 被 403 Authentication required 拒绝；保留鉴权，改从原生 UI
   配置仅 loopback 的模拟供应商。没有提取或绕过桌面访问凭据。
5. 开发 updater 访问专用无更新源的 localhost:59999 失败，属于开发启动日志中的已知错误。
   UI 测试保留 React/Happy DOM 警告，构建保留 Rust 未使用入口及 Vite 大 chunk 警告。

## 当前检查

| 检查 | 结果 |
| --- | --- |
| `bun run check` | 全链退出 0，含 TypeScript、桌面边界、i18n、主题、图标、CSS、平台边界、旧术语、help |
| Agent Engine UI 定向 7 文件 | 30 通过 / 0 失败，201 断言 |
| `cargo test --offline -p nomifun-engine-core -p nomifun-coding-engine --lib` | Core 14 + Coding 39 通过 |
| `cargo test --offline -p nomifun-app --test nomi_core_live_provider_smoke -- --test-threads=1` | 15 通过，3 项真实 Provider 入口 ignored |
| `cargo test --offline -p nomifun-db --lib database::displaced_conversation_runtime_migration::tests:: -- --test-threads=1` | 6 通过 |
| `cargo test --offline -p nomifun-desktop --bin nomifun-desktop macos_ -- --test-threads=1` | 5 通过 |
| dev/macOS build/helper 三组 Bun 脚本测试 | 21 通过 / 0 失败 |

旧报告中的 vocabulary 109 处失败已不适用于当前源码；本轮没有关闭或调整门禁。
所有 Cargo 构建串行。没有为了清理无关 warning 扩大实现范围。

## CONFLICT、执行及取消

历史 `engine.nomi.create` / `engine.coding.create` 标识首轮文件任务阶段，不等同 Session 创建接口。
历史根因是 canonical Agent 被合入不可变 Skill locks 之外的自动 Skills；当前创建、切换 Agent、
更新 capability-selection 三条防注入分支均保留，新 Skill loader 继续执行精确锁校验。

当前应用级 loopback 用例实际通过：双引擎首次文件写入、只读命令、多轮续接、Fork 精确绑定、
Agent 改引擎后旧会话不改绑、流中取消后续接，以及活动 `/bin/sh` 与 `sleep` 两代进程取消。
活动进程用例先证明 PID identity 存在，再等到会话 finished 且进程均消失；宿主正常关闭。
这些结果使用确定性本地模型服务，不能替代真实 Provider 验收。

本轮环境缺少用户安全提供的 `NOMIFUN_LIVE_STEPFUN_API_KEY`。真实模型、真实压缩与真实模型流中取消
均未运行；未寻找其他凭据、调用其他模型或 endpoint。后续复现入口为
`bun scripts/validation/run-nomi-core-live-provider-smoke.mjs --engine-smoke`，模型仅 `step-3.7-flash`。
前轮 StepFun 八阶段/压缩成功仅作为 `2ede9aaf7` 阶段历史证据，不声称当前源码真实模型通过。

## 原生开发窗口

命令（相对证据根位于本仓库 `.git/macos-engine-70c28b5de`）：

```sh
CARGO_TARGET_AARCH64_APPLE_DARWIN_RUNNER="$PWD/.git/macos-engine-70c28b5de/dev-app-runner.py" \
NOMIFUN_DATA_DIR="$PWD/.git/macos-engine-70c28b5de/dev-data" \
bun run dev --no-watch
```

测试用 Cargo runner 仅为未修改的 debug 主程序补临时 app 身份，SHA-256 字节一致性写入
`dev-app-runner-proof.json`；不引入额外执行器。开发地址 `localhost:5173/#/guid`，窗口首次正常显示，
新数据根初始会话为空，backend 62965 的 `/health` 200。显式数据根 WebView store 的回归已通过。

通过原生 UI 完成：

- 仅配置本地模拟供应商与 `step-3.7-flash`，明确选择 streaming/function_calling/reasoning；
  实际 provider URL 为 127.0.0.1，使用模拟占位凭据，没有真实云端调用。
- 官方最简问答模板发现两个官方引擎，选择 Coding 后保存为“macOS 当前基线 Agent”。
  使用 Agent 后首页保留选中的个人 Agent，原生发送 Coding/Nomi 均显示 `native-ui-binding-ok`。
- 个人 Agent 基本设置重新载入 Coding，改为 Nomi 并保存，新会话采用 Nomi；
  原 Coding 会话仍可续接。数据库前后对比所有旧 exact binding 均未改变，三个会话均 finished。
- Command-Q 正常关闭，dev runner 退出 0；app/runner/Vite 无残留，后端及 Vite 端口释放，
  日志有 runtime shutdown、plugins shutdown 和 terminal cleanup。

原生 UI / 数据库绑定证据：`dev-evidence.json`、`bindings-before.json`、`bindings-after.json`。

| 引擎 | 当前 build | 当前 digest |
| --- | --- | --- |
| Nomi | `0.7.6-host62` | `969e19da46766957a5489cc6a54483c868d31f14335211dea9248c5cc247baa1` |
| Coding | `0.7.6-host2-coding-loop95` | `c7a76c3c9bca22b70e1c6618bf73a6057e84d550728a85a03ece82f67a0510aa` |

当前源码与前轮不同，构建摘要也不同；本轮没有改写已有会话的精确绑定。

## 当前原生 app / DMG

`bun run build:mac arm` 退出 0。Rust release 编译 10m15s，仅构建 Apple Silicon 原生架构。
构建时源码干净，release-lock v2 对应 `6125f1a6453f915f1dfcdd5d4543f8d45182c5d2`。
后续交接文档提交不改变该制品的源码归属。

- app：`target/aarch64-apple-darwin/release/bundle/macos/NomiFun.app`
- DMG：`dist/desktop/NomiFun_0.7.6_aarch64.dmg`，88,448,834 字节。
- `hdiutil verify` 通过；只读挂载 DMG 后，主程序与构建 app 字节一致，arm64、可执行权限存在。
- 全部 **404** 个前端文件路径和 SHA-256 与本轮 `ui/dist` 一致；LICENSE / NOTICE 一致。
  原打包脚本的退休执行器资源拒绝检查通过，没有外部 Wrapper / hello 残留。
- 原生 helper 总结果 **fail**，唯一失败项为严格 `codesign`：
  `code has no resources but signature indicates they must be present`。
  主程序的 linker ad-hoc 不等于完整 bundle 签名，未降低门禁。
- 没有执行 Developer ID 签名、公证、Gatekeeper 安装/升级/卸载，未发布 Release 或更新。

| 制品 | SHA-256 |
| --- | --- |
| DMG | `b180461069336f3aaa025efd3699f9dd5a5c6e95b240cf12df5112824a9a0683` |
| app 主程序 | `833da569de6147c7af6e0a6d45ff06da3d5607df08167e7c913acb7fb0f64a84` |
| release-lock | `24af389bffc728311ccd52726c14d5796162fdefc14e97cbb581bb1a3860ca7c` |

### 包内启动、交互及退出

从 DMG 只读挂载的 `NomiFun.app/Contents/MacOS/nomifun-desktop` 启动，
`NOMIFUN_DATA_DIR` 指向本轮开发验证用的隔离根。没有启动 Vite 或使用已安装应用。
backend 63805 的 `/health` 返回 200，启动后原有三个会话与 exact binding 保持不变。

首次尝试观察时 Mac 锁定，CUA 无法自动解锁；用户明确回复“已解锁”后继续完成：

- 原生窗口地址为 `tauri://localhost#/guid`，个人 Agent 及模型选择正确恢复。
- 包内新 Nomi 会话 `01a0a46d-26b7-7732-8890-71134bde8a9b` 完成；
  原 Coding 会话 `01a0a45f-edbc-7522-acad-998950e1cd8b` 在重启后继续发送并完成。
  均显示本地模拟回复 `native-ui-binding-ok`，非真实 Provider 证据。
- 通过原生“新建终端”明确输入 `/bin/sh -c 'sleep 120 & wait'`。
  退出前已观察 app 2033、watchdog 2375、shell 2376、sleep 2377 均存在。
- Command-Q 后应用退出码 0，上述全部 PID 消失，端口释放；日志包含正常 plugin/runtime shutdown
  和 `terminal shutdown cleanup ... deleted=1`。
- 关闭后确认数据库无 WAL/SHM，再以 immutable 只读模式比对：四个会话均 finished，
  旧 exact binding 不变，`terminal_sessions` / `terminal_scrollback` / `terminal_turn_admissions` 均为 0。
  原普通 `mode=ro` 辅助读取在关闭后报 unable to open database file，该辅助读取失败单独保留，
  改用已静止、无 WAL 的快照读取；没有写入数据库或伪造退出记录。
- DMG 已卸载，loopback 模拟服务按 SIGTERM 结束；测试 app、开发 runner、端口均已清理。
  数据、日志和旧制品备份保留，既有 `dist/desktop/latest.json` SHA-256 不变。

证据：`package-qa.json`、`package-native-report.json`、`package-run.log`、
`package-active-processes.json`、`package-process-cleanup.json`、`package-bindings-final.json`。
本轮没有重复执行包内官方模板保存：该操作已在同源码开发窗口实测，包内通过资源逐字节比对及个人 Agent/会话交互验证。

## 收尾结论与剩余项

**当前 macOS 原生开发与本地包可复现，双引擎的确定性执行、精确绑定、取消和原生正常退出已有当前证据；
工程 `bun run check` 通过。严格 bundle 签名失败、当前源码真实 StepFun 测试缺凭据，不能宣布正式发布验收全绿。**

仍未验证：当前源码真实 StepFun 双引擎八阶段、真实压缩、真实 Provider 流中取消；异常退出恢复；
真实用户数据库迁移；Windows 当前公共改动原生回归；Linux 原生运行；正式安装/升级/卸载。
历史 Coding 重试经 rewind 返回不支持的限制未重测，本轮未新增 rewind。
不恢复 Wrapper、不新增社区 Engine 验收，不做 SDK 重构，不修改历史持久化编码。

最后 fetch 时 origin 仍为 `70c28b5de`，源码修复仅本地 ahead 1 / behind 0；后续文档提交也仅留本地。
没有 reset、强推、覆盖其他人的改动、推送、签名、公证或发布。
