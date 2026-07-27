# Browser Platform 收尾交接、TODO 与启动 Prompt

状态：`CORE IMPLEMENTED / LOCAL WINDOWS RELEASE VERIFIED / FOLLOW-UP OPEN`

更新时间：2026-07-27
集成基线：`integrate/browser-platform-main-work`，实现提交基线 `76f2bdca`

本文是 Browser Platform 后续工作的权威交接入口。它记录已经完成的范围、
仍未完成的事项和可直接交给下一位开发者或 Agent 的启动 Prompt。这里的
TODO 不能被理解为核心实现缺失；当前核心已经完成并通过 Windows 本地发布
验收，剩余工作主要是跨平台、发布流程、长期运维和产品/安全决策闭环。

> 仓库强制规则：`.github/workflows/` 下不得存在任何 `.yml` 或 `.yaml`。
> 不得创建、恢复、生成或启用 GitHub Actions。所有构建、测试和发布验证
> 必须使用本地脚本与人工记录。

## 1. 已完成，不要重复实现

- 主进程唯一 `BrowserSessionHub`、Browser Host/Lane、身份域、资源调度、
  owner/control/viewer lease、inventory、清理与恢复已经落地；
- Native、Gateway、ACP/Codex stdio、remote 与 cluster attempt 已统一进入
  Hub；ACP browser stdio 是作用域代理，不再拥有 Chromium；
- Browser 管理页、设置、嵌入式 Viewer、用户接管、归还控制、Lane/
  conversation/全局关闭与 realtime inventory 已落地；
- 旧 display-mode 配置迁移、稳定错误契约、CSRF、Origin、单次 viewer
  token、敏感元数据过滤已经落地；
- dataset recovery 与 remote `main` 截至 `11eee85e` 的修复已并入实现基线；
- 当前生产架构见
  [`../architecture/browser-platform.zh.md`](../architecture/browser-platform.zh.md)，
  用户/Agent 使用约定见
  [`../guides/computer-browser-use.zh.md`](../guides/computer-browser-use.zh.md)。

### 已记录的验证证据

- `bun run check`、`bun run typecheck`、`bun run build:ui`；
- `cargo check --workspace --all-targets` 与 `cargo fmt --all -- --check`；
- Browser UI/Viewer 102 项、Adapter 22 项、Browser Platform 146 项、
  Realtime 89 项、App WebSocket E2E 18 项；
- Windows 真实 Chromium：4 Lane 并发与同 Lane 串行、16 Lane、Viewer
  screencast、三轮共享 Host RSS；共享/独立 Host RSS 中位比 `0.4103`，
  约降低 `58.97%`，所有轮次 residual PID 为空；
- Windows x64 生产包和隔离数据目录 release smoke；`GET /health` 返回
  `200`，托盘 Quit 后桌面进程、监听端口和受管浏览器无残留。

本机的忽略目录日志只用于本次审计，不是可移植的源码真相。换机后应按本文
门禁重新生成证据，不能只引用旧日志宣称通过。

## 2. 尚未完成的 TODO

### P0：合并或发布前必须完成

- [ ] 在目标机器从**最新远程 `main`** 新建/更新本地集成基线，重新核对
  `origin/main...HEAD`。如远程继续变化，先把远程 `main` 合入本地分支，
  解决冲突后重新执行受影响门禁；禁止直接 push 到远程 `main`。
- [ ] 执行并记录 workspace 全量 `cargo test`（当前只记录了 all-targets
  check 和大量定向测试，尚没有完整 workspace `cargo test` 通过记录）。
- [ ] 执行完整 UI 测试矩阵，至少覆盖现有 Browser、Viewer、Adapter、
  设置迁移与 route/capability gating；不要只依赖 typecheck/build。
- [ ] 在正式签名/发布环境重新构建目标安装包并执行隔离数据目录 smoke。
  当前 Windows 安装包未做 Authenticode 签名，也没有 updater `.sig`；
  签名、updater metadata、安装/升级/卸载验证仍属于发布责任。
- [ ] 完成发布说明与用户告知：Browser 默认为 embedded、Primary 是
  “共享实时身份”、用户关闭 Lane 不会关闭 conversation/execution，且
  v3 dataset 升级/退休策略必须与
  [`06-open-questions.zh.md`](06-open-questions.zh.md) 保持一致。

### P1：跨平台和故障矩阵

- [ ] 在 macOS 和 Linux 的 bundled/configured Chromium 上复跑真实浏览器
  验收：跨 Lane 重叠、同 Lane 串行、16 attempt、Viewer、RSS、debug
  endpoint 与进程残留；记录硬件、OS、Chromium 版本和时间线。
- [ ] 验证 Windows Job Object、Unix parent-death pipe 和显式 Host shutdown
  的真实崩溃/强退边界，而不仅是正常托盘退出。
- [ ] 建立 `Page.setWebLifecycleState`、`Page.startScreencast` 与截图降级的
  Windows/macOS/Linux 能力矩阵，记录不支持时的冻结/重建行为。
- [ ] 补齐 target crash、Crawl Host crash、Primary Host restart/circuit
  breaker 的真实 Chromium 故障注入；证明旧 epoch/ref 确定失效且故障
  不跨 Host/Lane 扩散。
- [ ] 对 ACP capability expiry、remote disconnect、attempt/runtime 终止、
  conversation 删除与应用关闭执行端到端生命周期验收，确认 5 秒内释放容量。
- [ ] 完成 dataset reset 在真实文件系统上的中断、双锁、权限拒绝、磁盘不足、
  恢复与产品交互矩阵；同时关闭
  [`06-open-questions.zh.md`](06-open-questions.zh.md) 中仍开放的 v3 发布门禁。

### P2：产品、安全和长期运维决策

- [ ] 用安全评审确认 Authenticated Replica snapshot 实际覆盖范围：cookies、
  localStorage、IndexedDB、service worker、设备绑定凭据和 passkey；不得
  对无法安全复制的身份能力作过度承诺。
- [ ] 维护并评审“可能修改身份或持久账户状态”的 Browser action 清单，
  确保 Replica 始终路由 Primary 或返回 `needs_primary_identity`。
- [ ] 用固定硬件基准确认 Automatic、Resource saving、High concurrency
  三档倍率和高级自定义边界，并记录各平台合理默认值。
- [ ] 完成 remote capability 的签发、续期、撤销和审计策略评审。
- [ ] 为真实 Chromium/RSS 验收建立可重复的本地 fixture 运行手册和结果
  归档格式；不得依赖外部网站，也不得借助 GitHub Actions。
- [ ] 决定 retired dataset 的保留天数、磁盘上限和显式 GC 入口；GC 不得
  扫描或删除用户拥有的外部 workspace。
- [ ] 根据真实用户反馈继续做 Browser 页可用性/无障碍/多显示器 Viewer
  验收，但不要以复制页面、iframe 或第二会话替代真实 target。

## 3. 继续工作前的安全护栏

1. 先读根目录 `AGENTS.md`；任何情况下都不得创建 GitHub Actions workflow。
2. 不要 `reset --hard`、`clean -fdx` 或覆盖来源不明的脏工作区。
3. 真实 Chromium 只使用临时 profile；不得读取、复制或终止用户个人
   Chrome/Edge profile 或进程。
4. 所有 desktop smoke 设置新的 `NOMIFUN_DATA_DIR`。桌面壳会在该目录下
   再追加 `Nomi`，所以每轮使用全新的父目录。
5. 正常关闭必须通过应用退出路径并等待显式 Host shutdown；强杀不能作为
   shutdown 验收通过的证据。
6. `release:*` 脚本可能 tag、上传或 push；未经用户明确授权不得运行。
7. 错误、日志和文档不得记录 cookie、站点存储、raw CDP endpoint、
   debugging port、profile path、token、密钥或真实用户内容。

## 4. 推荐验证顺序

先做廉价门禁，再做重型或真实浏览器验收：

```bash
git status --short --branch
git fetch origin main
git rev-list --left-right --count origin/main...HEAD
git diff --check

bun run check
bun run typecheck
bun run build:ui
cargo check --workspace --all-targets
cargo test
cargo fmt --all -- --check
```

然后执行 Browser/Viewer/Adapter/Realtime/App WebSocket 定向测试，以及
`crates/agent/nomi-browser-engine/tests/integration_managed_host.rs` 中四个
`#[ignore]` 真实 Chromium 验收。设置 `NOMIFUN_CHROME_BINARY`，使用本地
fixture、临时 profile，并以单线程运行 RSS 项。最后执行目标平台生产构建、
隔离数据目录启动、`/health`、正常退出和残留检查。

每轮结束都必须执行：

```powershell
git status --short
git diff --check
Get-ChildItem .github/workflows -Recurse -File |
  Where-Object { $_.Extension -in '.yml', '.yaml' }
```

最后一条必须输出零个文件。

## 5. 可直接复制的启动 Prompt

```text
你正在继续 NomiFun Browser Platform 的发布收尾工作。

第一步必须阅读：
1. 根目录 AGENTS.md；
2. docs/continuity/07-browser-platform-handoff.zh.md；
3. docs/architecture/browser-platform.zh.md；
4. docs/guides/computer-browser-use.zh.md；
5. docs/continuity/06-open-questions.zh.md。

先只读审计当前机器：输出 git status、当前分支/HEAD、worktree 列表、
origin/main 与 HEAD 的 ahead/behind、现有构建工具和可用 Chromium。先执行
git fetch origin main，但不要 push。若工作区脏，保留所有现有改动，不要
reset/clean；创建独立 worktree 或安全分支继续。

核心 BrowserSessionHub/Host/Lane、统一入口、资源调度、身份域、Viewer、
Browser UI/API、lease 和清理已经实现，不要重写。按交接文档 TODO 从 P0
开始，优先关闭 workspace 全量测试、完整 UI 测试、最新 main 集成和目标
平台发布 smoke，再处理 P1/P2。可以用多个 Agent 并发处理互不重叠的只读
审查、跨平台验证和文档工作，但关键合并与有风险的进程/文件操作由主线程
执行。

绝对约束：
- .github/workflows 下禁止任何 yml/yaml，禁止创建或启用 GitHub Actions；
- 不得 push 到远程 main，不得未经授权运行可能 tag/upload/push 的 release 脚本；
- 不得读取或终止用户 Chrome/Edge；真实 Chromium 只用临时 profile；
- desktop smoke 必须使用全新的 NOMIFUN_DATA_DIR；
- 不得暴露 cookie、存储值、CDP endpoint、debug port、profile path 或密钥；
- 不得用强杀冒充正常 shutdown 验收。

每完成一项都记录命令、结果、平台/硬件、残留进程与端口检查。只有所有
P0 门禁有新机器上的证据后，才可宣称可发布；失败时保留可复现日志并返回
稳定原因，不要静默跳过。
```

## 6. 完成判定

本文 TODO 完成不是“代码能编译”。至少要有：最新 `main` 集成、workspace
全量测试、完整 UI 矩阵、目标平台真实 Chromium、签名/安装/升级/卸载、
正常退出无残留、安全评审和发布说明的可审计证据。任何一项未完成都应在
交接中明确保留，不得把历史机器上的忽略日志当作新平台通过。
