# Browser Platform 收尾交接、TODO 与启动 Prompt

状态：`AGENT-ONLY CONTRACT CURRENT / CORE IMPLEMENTED / FOLLOW-UP OPEN`

更新时间：2026-07-27
集成基线：`integrate/browser-platform-main-work`，实现提交基线 `76f2bdca`

本文是 Browser Platform 后续工作的权威交接入口。它记录现行产品契约、已经
完成的范围、仍未完成的事项和可直接交给下一位开发者或 Agent 的启动 Prompt。
这里的 TODO 不能被理解为核心 Hub/Lane 实现缺失；剩余工作主要是按 Agent-only
方向重跑门禁、跨平台验证、发布流程、长期运维和产品/安全决策闭环。

> **2026-07-27 取代性产品决策（superseding decision）：**早期嵌入式
> Viewer、JPEG/screencast、专用 Viewer WebSocket、viewer token、用户接管与
> 交还控制已经退出产品并从生产入口移除。与此冲突的旧设计、测试说明和发布
> 叙述均为 **superseded（已取代）历史**，不得解释为当前能力或待恢复功能。
> 普通 Agent Browser Use 现在以 Chromium `--headless=new` 运行 Primary，不创建
> 操作系统窗口。当前“前台打开”会安全关闭旧 headless Host，并用应用托管 profile
> 创建 headful 替代 Host；它不恢复上述 Viewer、preview、接管、交还控制或页面
> 输入能力。旧“Primary 始终可见/自动弹窗/最小化后台”叙述均已被该契约取代。

> 仓库强制规则：`.github/workflows/` 下不得存在任何 `.yml` 或 `.yaml`。
> 不得创建、恢复、生成或启用 GitHub Actions。所有构建、测试和发布验证
> 必须使用本地脚本与人工记录。

## 0. 现行产品契约（后续工作以此为准）

- Browser 是 **Agent-only 受管浏览器**。普通 Agent Browser Use 使用
  `--headless=new` 的 Primary Chromium，不创建窗口、不弹窗或抢焦点。
  `/browser` 页面仅展示和管理 Lane/Host 的状态、容量/队列、身份与生命周期；
  不承载页面内容或页面执行。
- 用户可在 `/browser` 对自己拥有的 running Primary Lane 显式“前台打开”。认证
  接口为 `POST /api/browser/lanes/{id}/foreground`；它会安全关闭旧 headless
  Host，并使用同一应用托管 profile 创建 headful 替代 Host，也不成为 Agent action。
  browser epoch 随进程替换而变化，旧 target/frame/ref 必然失效；系统只尽力恢复
  Lane 的活动 URL，调用方必须刷新库存并 fresh observe。
- Browser 页面/API 不提供嵌入 JPEG/screencast、专用 Browser Viewer
  WebSocket、用户页面输入、接管/交还控制、viewer token 或第二浏览器会话
  接口。旧
  `/api/browser/lanes/{lane_id}/return-control`、`.../viewer-token` 和
  `.../view` 路由属于 superseded 历史，客户端不得调用，后续工作不得恢复。
- 仅安装 owner 可用的 `/api/browser/login/open`、`.../close`、`.../status`
  是 Primary 登录兼容生命周期入口：显式 login-open 会前台打开同一 Hub 中的
  普通 Primary Lane（包括复用已有 Lane），不创建嵌入式或第二浏览器表面，也不
  向 Browser 管理页/API 增加页面输入控件。
- Primary 始终使用 NomiFun 管理的应用隔离稳定 profile；普通 Agent Host 真正
  headless，显式前台流程才创建 headful 替代 Host。不得复用用户个人 Chrome/Edge
  profile。Crawl/Anonymous、Authenticated Replica 与 Isolated Host 继续由 Hub
  按策略 headless 运行。
- raw CDP endpoint、debugging port、profile 路径与 profile 内容均为 Hub 内部
  实现细节，不得通过 UI、API、Agent 工具、realtime 事件、错误或日志暴露。
- 主进程唯一 `BrowserSessionHub`、Browser Host/Lane、同 Lane 串行与跨 Lane
  并发、资源调度、身份路由、owner lease、清理和显式关闭语义继续保留。
- Native Agent turn 正常结束或取消时关闭该 turn owner 的 Lane；Lane 完成 target
  清理后，如果它是所属 Host 的最后一个 Lane，Host 必须立即退出，不等待 idle
  expiry、周期 sweep 或 warm timer。
- Browser 页面边界改变不影响 Agent 安全边界。可能改变页面、账户或外部状态的
  动作继续经过应用审批；高风险或不可逆动作以及审批旁路场景保持 fail-closed。
- 通用 `/ws` 仍是已鉴权 JSON realtime 通道，继续承载 Browser inventory/
  生命周期等应用事件；它不是页面 Viewer 或输入通道，不得因移除 Viewer 而删除。

## 1. 已完成，不要重复实现（现行）

- 主进程唯一 `BrowserSessionHub`、Browser Host/Lane、身份域、资源调度、
  owner lease、inventory、清理与恢复已经落地；
- 同一 Lane 操作串行、不同 Lane 有界并发，以及容量排队和资源策略已经落地；
- Native、Gateway、ACP/Codex stdio、remote 与 cluster attempt 已统一进入
  Hub；ACP browser stdio 是作用域代理，不再拥有 Chromium；
- Browser 状态管理页、默认 `--headless=new` 且可显式替换为 headful Host 的应用
  隔离 Primary、`POST /api/browser/lanes/{id}/foreground`、Lane/conversation/全局关闭与
  realtime inventory 已落地；管理页不提供页面内容、页面输入或用户接管；
- 旧 display-mode 配置统一迁移到 `external`，稳定错误契约、CSRF 和敏感元数据
  过滤已经落地；旧 `agent.browserUse.silent` 只读不写；
- Agent 动作分类、常规审批与高风险/不可逆动作的 fail-closed 红线继续保留；
- dataset recovery 与 remote `main` 截至 `11eee85e` 的修复已并入实现基线；
- 当前生产架构见
  [`../architecture/browser-platform.zh.md`](../architecture/browser-platform.zh.md)，
  用户/Agent 使用约定见
  [`../guides/computer-browser-use.zh.md`](../guides/computer-browser-use.zh.md)。

### 历史验证记录（Viewer 部分已 superseded）

以下是产品方向变更前后在既有实现基线记录的本地证据，不等同于当前工作树的
Agent-only 发布验收。凡涉及 Viewer、JPEG、screencast、接管或 viewer token 的
项目仅供追溯，已经 superseded，不得作为现行能力证明，也不得为复跑而恢复入口。

- `bun run check`、`bun run typecheck`、`bun run build:ui`；
- `cargo check --workspace --all-targets` 与 `cargo fmt --all -- --check`；
- 历史 Browser UI/Viewer 合计 102 项（其中 Viewer 项已 superseded）、Adapter
  22 项、Browser Platform 146 项、Realtime 89 项、App WebSocket E2E 18 项；
- Windows 真实 Chromium：4 Lane 并发与同 Lane 串行、16 Lane、历史 Viewer
  screencast 实验（已 superseded）、三轮共享 Host RSS；共享/独立 Host RSS
  中位比 `0.4103`，约降低 `58.97%`，所有轮次 residual PID 为空；
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
- [ ] 执行完整 UI 测试矩阵，至少覆盖 Browser 状态管理、running Primary 的显式
  前台打开、非 running/非 Primary 的 fail-closed、Adapter、设置迁移与
  route/capability gating；确认已取代的旧 Viewer/接管/token 路由未挂载且客户端
  不再调用；不要只依赖 typecheck/build。
- [ ] 在正式签名/发布环境重新构建目标安装包并执行隔离数据目录 smoke。
  当前 Windows 安装包未做 Authenticode 签名，也没有 updater `.sig`；
  签名、updater metadata、安装/升级/卸载验证仍属于发布责任。
- [ ] 完成发布说明与用户告知：普通 Agent Browser Use 默认以 `--headless=new`
  运行 Primary，不创建窗口；用户可对 running Primary 显式请求 headful 替代
  Host，显式登录流程也会请求前台 Host。进程替换会改变 epoch、使旧 ref 失效，
  URL 只尽力恢复且后续必须 fresh observe；应用隔离 profile 与“共享实时身份”不变，
  `/browser` 仍只管理状态/容量/身份/生命周期，不恢复 preview/接管/input，非
  Primary Host 保持 headless。用户关闭 Lane 不会关闭 conversation/execution，
  但 turn/Lane 清理后的最后一个 Host 会立即退出；
  Agent 高风险审批仍保留；v3 dataset 升级/退休策略必须与
  [`06-open-questions.zh.md`](06-open-questions.zh.md) 保持一致。

### P1：跨平台和故障矩阵

- [ ] 在 macOS 和 Linux 的 bundled/configured Chromium 上复跑真实浏览器
  验收：跨 Lane 重叠、同 Lane 串行、16 attempt、RSS、debug
  endpoint 与进程残留；不得把 debug endpoint 作为产品 API 或客户端能力暴露，
  记录硬件、OS、Chromium 版本和时间线。
- [ ] 分平台确认普通 Primary 以 `--headless=new` 和应用隔离稳定 profile 启动，
  不创建窗口、弹窗或抢焦点；显式前台请求安全关闭旧 Host、创建 headful 替代
  Host、递增 epoch，并尽力恢复 Lane URL。确认旧 ref 失效且必须 fresh observe；
  显式登录会请求前台 Host。并确认 Crawl/Anonymous、Authenticated Replica、
  Isolated 的 headless 选择不改变身份隔离、容量或关闭语义。
- [ ] 验证 Windows Job Object、Unix parent-death pipe 和显式 Host shutdown
  的真实崩溃/强退边界，而不仅是正常托盘退出。
- [ ] 建立 `Page.setWebLifecycleState` 的 Windows/macOS/Linux 能力矩阵，
  记录不支持时的冻结/重建行为；不要恢复已取代的 Viewer screencast。
- [ ] 补齐 target crash、Crawl Host crash、Primary Host restart/circuit
  breaker 的真实 Chromium 故障注入；证明旧 epoch/ref 确定失效且故障
  不跨 Host/Lane 扩散。
- [ ] 对 ACP capability expiry、remote disconnect、attempt/runtime 终止、
  conversation 删除与应用关闭执行端到端生命周期验收，确认 5 秒内释放容量。
  另外验证 Native Agent turn 正常完成/取消会关闭 owner Lane，显式关闭 Lane 会先
  清理 target，并让最后一个 Host 立即退出而不等待 sweep/warm timer。
- [ ] 完成 dataset reset 在真实文件系统上的中断、双锁、权限拒绝、磁盘不足、
  恢复与产品交互矩阵；同时关闭
  [`06-open-questions.zh.md`](06-open-questions.zh.md) 中仍开放的 v3 发布门禁。

### P2：产品、安全和长期运维决策

- [ ] 用安全评审确认 Authenticated Replica snapshot 实际覆盖范围：cookies、
  localStorage、IndexedDB、service worker、设备绑定凭据和 passkey；不得
  对无法安全复制的身份能力作过度承诺。
- [ ] 维护并评审“可能修改身份或持久账户状态”的 Browser action 清单，
  确保 Replica 始终路由 Primary 或返回 `needs_primary_identity`。
- [ ] 对 desktop、Gateway、remote 与审批旁路会话复跑 Agent 高风险/不可逆
  Browser action 矩阵，确认审批、带外确认和 hard-deny 保持 fail-closed；不得
  以“已前台打开真实窗口”替代应用审批。
- [ ] 用固定硬件基准确认 Automatic、Resource saving、High concurrency
  三档倍率和高级自定义边界，并记录各平台合理默认值。
- [ ] 完成 remote capability 的签发、续期、撤销和审计策略评审。
- [ ] 为真实 Chromium/RSS 验收建立可重复的本地 fixture 运行手册和结果
  归档格式；不得依赖外部网站，也不得借助 GitHub Actions。
- [ ] 决定 retired dataset 的保留天数、磁盘上限和显式 GC 入口；GC 不得
  扫描或删除用户拥有的外部 workspace。
- [ ] 根据真实用户反馈继续做 Browser 状态管理页的可用性和无障碍验收；
  不要恢复图片轮询、iframe、第二会话或用户接管入口。

## 3. 继续工作前的安全护栏

1. 先读根目录 `AGENTS.md`；任何情况下都不得创建 GitHub Actions workflow。
2. 不要 `reset --hard`、`clean -fdx` 或覆盖来源不明的脏工作区。
3. 真实 Chromium 验收只使用新建的应用隔离测试 profile；不得读取、复制或
   终止用户个人 Chrome/Edge profile 或进程。生产 Primary 的稳定应用隔离
   profile 语义不得因此改成临时 profile。
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

然后执行 Browser 状态管理/Adapter/Realtime/App WebSocket 定向测试；通用
`/ws` 的鉴权、JSON 事件与 Browser inventory/生命周期事件仍必须覆盖。对
`crates/agent/nomi-browser-engine/tests/integration_managed_host.rs`，现行真实
Chromium 验收只包括并发矩阵、16 Lane 与 RSS 三类；文件中历史
`managed_host_viewer_screencast_real_chromium_acceptance` 已 superseded，不是
当前发布门禁，也不得作为恢复 Viewer 的理由。设置 `NOMIFUN_CHROME_BINARY`，
使用本地 fixture、临时应用隔离 profile，并以单线程运行 RSS 项。最后执行
目标平台生产构建、隔离数据目录启动、`/health`、正常退出和残留检查。

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

现行产品是 Agent-only 受管浏览器。普通 Agent Browser Use 默认以 Chromium
`--headless=new` 运行 Primary，不创建窗口、弹窗或抢焦点。Browser 页面仅管理
状态、容量、身份与生命周期；用户可对 running Primary 显式“前台打开”，通过
`POST /api/browser/lanes/{id}/foreground` 安全关闭旧 headless Host，并用同一应用
托管 profile 创建 headful 替代 Host。进程替换会递增 epoch，旧 target/frame/ref
失效；系统只尽力恢复 Lane URL，后续必须刷新库存并 fresh observe。显式登录流程
也会请求前台 Host。没有嵌入 JPEG/screencast、专用 Viewer WebSocket、页面输入、
用户接管/交还控制或 viewer-token 接口，也不得借前台能力恢复这些入口。Primary
使用应用隔离 profile；Crawl/Anonymous、Authenticated Replica、Isolated 保持
headless。通用 `/ws` realtime 继续保留，但不是 Viewer。Agent turn 正常结束或
取消会关闭 owner Lane；关闭最后一个 Lane 后 Host 立即退出，不等待周期清理。

核心 BrowserSessionHub/Host/Lane、统一入口、同 Lane 串行/跨 Lane 并发、资源
调度、身份域、状态管理 UI/API、owner lease、清理与显式关闭已经实现，不要
重写。Agent 高风险和不可逆动作仍须经过现有审批并保持 fail-closed。不要恢复
任何 superseded Viewer 或用户接管能力。按交接文档 TODO 从 P0 开始，优先关闭
workspace 全量测试、完整 UI 测试、最新 main 集成和目标平台发布 smoke，再处理
P1/P2。可以用多个 Agent 并发处理互不重叠的只读审查、跨平台验证和文档工作，
但关键合并与有风险的进程/文件操作由主线程执行。

绝对约束：
- .github/workflows 下禁止任何 yml/yaml，禁止创建或启用 GitHub Actions；
- 不得 push 到远程 main，不得未经授权运行可能 tag/upload/push 的 release 脚本；
- 不得读取或终止用户 Chrome/Edge；真实 Chromium 验收只用新建的应用隔离
  测试 profile，生产 Primary 仍使用稳定应用隔离 profile；
- desktop smoke 必须使用全新的 NOMIFUN_DATA_DIR；
- 不得暴露 cookie、存储值、CDP endpoint、debug port、profile path 或密钥；
- 不得用强杀冒充正常 shutdown 验收。

每完成一项都记录命令、结果、平台/硬件、残留进程与端口检查。只有所有
P0 门禁有新机器上的证据后，才可宣称可发布；失败时保留可复现日志并返回
稳定原因，不要静默跳过。
```

## 6. 完成判定

本文 TODO 完成不是“代码能编译”。至少要有：最新 `main` 集成、workspace
全量测试、完整 UI 矩阵、Agent-only 路由/能力边界、通用 `/ws` realtime、
Primary 默认 `--headless=new`、显式前台 Host replacement、epoch/ref 失效与
fresh observe、登录流程前台打开、turn/Lane 后最后 Host 立即退出、应用隔离
profile、非 Primary headless 策略、Agent 高风险审批、目标平台真实 Chromium、
签名/安装/升级/卸载、正常退出
无残留、安全评审和发布说明的可审计证据。任何一项未完成都应在交接中明确保留；不得把
历史 Viewer 测试或旧机器上的忽略日志当作现行能力或新平台通过。
