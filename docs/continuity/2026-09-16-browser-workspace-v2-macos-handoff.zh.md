# Browser Workspace v2：macOS 实施移交

> 状态：Windows 完成后执行；本文不声称 macOS 已支持。
>
> 基线：`docs/specs/2026-09-13-browser-workspace-v2.zh.md` 与
> `docs/specs/2026-09-15-system-browser.zh.md` 的 2026-09-16 冻结合同。

## 1. 不可改变的产品合同

- 会话 Browser 必须是同一个真实原生 `WKWebView`；用户看到的页面就是 Agent 操作的页面。
- 不使用 iframe、JPEG/PNG 连续帧、screencast、Canvas 远程桌面或另一个 Headless 页面冒充。
- 只有 `UserReady` 与 `AgentRunning` 两态。Agent 工作时用户不能操作；原子动作 settle 后才恢复用户输入。
- 不增加 takeover/交还、测试步骤、问题、终端、控制台、DevTools/F12、popout/归位或 Browser 设置中心。
- `nomi_local_websearch` 继续使用隔离 Headless Search owner；不得读取会话 Browser 登录态。
- `nomi_system_browser` 在没有经过验证的 macOS attach-only 路径前必须显示 unavailable；不得扫描、复制或启动用户
  默认浏览器 Profile 作为降级。
- macOS 失败时明确 unavailable；禁止退回 DOM `click()`/`dispatchEvent()`/直接改 value 或帧流。

## 2. 最小实现切片

1. 在 Tauri 主窗口内挂载真实 child `WKWebView`/`NSView`，复用现有 Conversation Workspace、Tab、revision、
   RunGuard 与 renderer slot 合同。页面 WebView 不获得 Tauri capability、local trust 或 backend credential。
2. 为每个 Conversation 建立应用拥有的网站数据边界。先用最小签名 app 实测目标 macOS/WebKit 的持久 data-store
   能力，再固定实现；不能悄悄共用系统 Safari 数据，也不能借用默认全局 store 后声称会话隔离。
3. 实现宿主内部语义观察桥，只返回当前已绑定页面的有界 accessibility/DOM 语义。WKWebView 没有可假定的公开 CDP；
   不把 Web Inspector 当生产控制接口。
4. 用 AppKit/WebKit 可公开验证的原生事件路径实现 click、hover、type、press、wheel 与同文档 drag。每种操作先在
   真实 fixture 证明默认行为、focus、IME 和 `event.isTrusted`；无法证明的动作返回 unsupported。
5. 实现 native input gate：run accepted 后立即阻断用户鼠标、键盘、拖放与触控板输入，同时保留 Agent 的宿主输入
   路径；Stop 期间保持锁定，settle 后再恢复 first responder 与用户输入。
6. 接入 popup、网站 dialog、permission、file chooser、download、crash、close 与 app shutdown。只实现首发流程所需
   的最小集合；没有证据的边界明确拒绝，不建设第二套管理平台。
7. Interactive Browser 沿用系统网络、DNS、代理、证书、localhost/LAN 与 WebSocket/HMR；仅校验顶层无凭据
   HTTP(S) URL 和拒绝特权 scheme。后台 Search/Render 继续使用自己的严格公网出口边界。
8. 站点数据清理必须只针对当前 Conversation 数据边界，先关闭该会话页面，再等待 WebKit 清理回执；失败不得显示成功。

## 3. 必须先做的原生 spike

- 签名最小 app 中：真实中文输入、组合文本、Tab/快捷键、hover、右键/双击、wheel、pointer/drag 默认行为。
- 同源 iframe、跨站 iframe、Shadow DOM、父层遮挡、页面导航后的旧 ref 失效。
- Agent 输入期间用户真实鼠标/键盘无法进入页面；停止/崩溃/窗口切换后不会留下按键或拖拽状态。
- hide/show、resize、Conversation 切换不重建页面；最小窗口仍遵守 880x600 桌面合同。
- 网站数据隔离与清除不会影响另一个 Conversation，也不会接触 Safari Profile。
- F12/Inspect/Web Inspector 没有产品入口。

若任何“原生输入”只能靠页面脚本合成事件完成，停止该动作的实现并记录 unsupported；不要扩大架构补偿。

## 4. 验收矩阵

- 复用 Windows fixture 语义：navigate/observe/click/type/press/wheel/select、同文档 drag、Stop、dialog、permission、
  popup、上传、下载、crash、close、shutdown 与站点数据清理。
- 验收必须记录页面侧 trusted 事件、默认行为、焦点与最终 DOM 状态，不能只看截图。
- 跑 renderer 定向测试、Rust 平台测试、`bun run check:desktop-ui-boundary`、
  `bun run check:browser-platform-boundary`、typecheck、i18n 与 macOS 签名包 smoke。
- 完成前 UI/文档保持 macOS unavailable；Windows 证据不能替代 macOS 证据。

## 5. 明确后置

- `nomi_system_browser` 的 macOS Safari/Chrome attach；只有官方、用户授权、attach-only 且不迁移 Profile 的路径
  通过后再启用。
- Linux/WebKitGTK。
- 跨进程 HTML drag、popout、DevTools、测试工作台与浏览器高级网络设置；这些不是本次移交任务。

## 6. 移交结果格式

接手方应返回：目标 macOS/WebKit/Tauri 版本、每个 native fixture 的通过/失败证据、明确 unsupported 的动作、
签名产物哈希与未运行项。不得用“编译通过”代替真实 WKWebView 输入和数据隔离证据。

## 7. 本次跨电脑交接基线

- 工作分支：`rf/agent-capability-platform-v2`，不是 `main`。
- Windows Browser 重构保全提交：`476b6bd0b`。
- 本次整合的远程基线：`a8b1685cf`。最终合并提交以包含本文的分支最新 Git 记录为准。
- 远程现已引入源码集成的多 Engine、新的 Agent authoring / Role defaults 和创作 UI。
  不要从旧 Windows 提交单独开工，不要还原已删除的 Fresh-v4 host / 旧 Browser Hub。
- 本次合并修复了 authoring 重新加载 Role defaults 时覆盖原生 Browser 绑定的问题。
  `NomiCoreRoleBindingStore` 必须保留当前宿主 materialize 出来的精确 Browser Role；无宿主不得伪造绑定。
- 这是代码交接，不包含 Windows 数据库、个人浏览器 Profile、登录态、API key、构建缓存或 EXE 的迁移。

## 8. 代码定位（相对仓库根目录）

| 点位 | 现状与 macOS 工作 |
| --- | --- |
| `apps/desktop/src/main.rs`、`browser_surface/mod.rs` | Windows 才注入原生 Browser host；macOS 需要真实宿主实现再启用，不能只放开 cfg。保留远程新增的 macOS 退出处理与 `ExplicitDesktopDataRoot`。 |
| `apps/desktop/src/browser_surface/{host,windows,automation,semantic_frames,frame_sessions}.rs` | Windows 参考实现；新增最小 WKWebView/AppKit 适配，复用公共合同，不复制第二套 workspace。 |
| `apps/desktop/src/browser_surface/{commands,security}.rs`、`apps/desktop/capabilities/default.json` | 原生 slot 定位、revision、主 UI 身份与不可信页面权限。验证 Retina/NSView 坐标和遮挡，不给网站 Tauri 权限。 |
| `crates/backend/nomifun-browser-platform/src/{runtime,workspace,run_guard,uploads,downloads}.rs` | 平台无关 workspace、tab、输入所有权与资源生命周期；优先复用。 |
| `crates/backend/nomifun-app/src/{desktop,services,browser_workspace_provider}.rs` | 宿主注入、能力 readiness、运行停止后再关 Browser 的顺序。 |
| `crates/backend/nomifun-app/src/router/{state,nomi_core_builtins,nomi_core_agent_projection,nomi_core_role_defaults,nomi_core_session}.rs` | Browser Role 声明、配置重编译、冻结快照及执行绑定。必须走正式配置→运行链路，不只测试手工注册工具。 |
| `crates/backend/nomifun-ai-agent/src/manager/nomi/{agent,browser_tool,browser_lifecycle}.rs` | Agent 工具与 RunGuard 生命周期；验证导航、观察、真实输入、Stop/失败释放。 |
| `crates/backend/nomifun-app/src/router/runtime_engines.rs`、`crates/backend/nomifun-ai-agent/src/coding_runtime.rs` | 远程新增 Engine 架构。Browser 当前接在 Nomi；不要把 Nomi 验收冒充 Coding Engine 验收，也不要在没有实现时暴露能力。 |
| `ui/src/renderer/pages/conversation/Browser/`、`components/ChatLayout/index.tsx`、`SystemBrowser/` | 复用会话侧边 Browser 和现有交互；检查宿主 availability 与 Cmd 快捷键。SystemBrowser 当前 `available={isWindowsRuntime}`，不能随内嵌 Browser 一起盲目启用。 |
| `crates/agent/nomi-browser-engine/src/{native_semantic,frame_geometry}.rs` | 可复用观察/坐标合同；WKWebView 不能假定存在 WebView2/CDP 接口。 |
| `apps/desktop/src/headless_browser_runtime.rs`、`crates/agent/nomi-browser-engine/src/{launch,headless_page}.rs` | 当前后台检索/Render 的 Windows runtime 引导依赖 Windows API；Mac 需独立探测应用可执行文件与版本。它不是会话 Browser 的降级实现。不要恢复旧 headless 用户产品。 |
| `crates/backend/nomifun-ai-agent/src/local_web_search/`、`crates/backend/nomifun-app/src/{headless_render.rs,router/knowledge_browser.rs}` | `nomi_local_websearch` 为 Agent 工作台可选能力，独立于原生页面；检索/后台渲染不得借用个人登录态。 |
| `crates/shared/nomifun-net/src/egress.rs`、`egress/public_dns.rs` | 后台公网出口边界；不要用于限制用户交互浏览器访问 localhost/LAN、系统代理或开发 HMR。 |
| `crates/agent/nomi-browser-engine/src/attached_browser.rs`、`crates/backend/nomifun-browser-platform/src/system_browser.rs`、app `router/system_browser.rs` | 另一路系统浏览器 attach-only；当前默认发现仅 Windows。Mac 首发未验证前 unavailable，不把启动临时 Chrome 当个人浏览器接入验收。 |
| `crates/backend/nomifun-agent-domain-wave1/src/lib.rs` | `nomi_system_browser` 当前目标白名单含 Windows x64 字面量；只有 Mac 实现和实机证据齐全才扩展。 |
| `vendor/wry/NOMIFUN-PATCH.md` | 当前补丁仅修改 WebView2 child close 所有权，WKWebView 未改。Mac 不可顺手删掉 Windows 补丁；如新增原生补丁，记录来源、原因与回归。 |
| `scripts/desktop-build-mac.sh`、`apps/desktop/tauri.macos.conf.json` | 本机 arm/intel/universal 构建与签名入口。使用源码集成 Engine，不恢复外部 Runtime 二进制。 |

## 9. 跨系统验证顺序与证据要求

1. **本机基线**：记录 `git rev-parse HEAD`、`sw_vers`、`uname -m`、`rustc -V`、`bun --version`、
   `xcodebuild -version`；先跑未改动基线，区分合并已有问题与 Mac 新问题。不要假定 Windows `target/` 可复用。
2. **最小原生闭环先行**：先证明会话内同一个 WKWebView 的 navigate/observe/click/type/press/wheel 和输入锁，
   再补生命周期。若当前公开 API 路径无法实现核心可信输入，应报告具体阻塞和最小替代决策，不能偷偷伪造事件。
3. **定向自动检查**（在仓库根目录；Cargo 串行，UI 检查可并行）：

   ```sh
   bun install --frozen-lockfile
   cargo check -p nomifun-desktop --bin nomifun-desktop --no-default-features
   cargo test -p nomifun-browser-platform
   cargo test -p nomifun-net --lib egress
   cargo test -p nomifun-app --features browser-use --lib nomi_core_role_defaults
   cargo test -p nomifun-app --features browser-use --test browser_workspace --test system_browser
   bun run check:browser-platform-boundary
   bun run check:desktop-ui-boundary
   bun run typecheck
   bun run check:i18n
   cargo run -p nomifun-agent-contracts --bin agent-v2-contract -- check
   ```

   Role 合同或 target manifest 有修改时先运行相同 generator 的 `write`，再运行 `check`；不手改生成摘要。
   定向 UI tests 参见 `Browser/`、`SystemBrowser/` 与 ChatLayout 相邻测试；不添加移动端测试。
4. **原生 fixture**：`apps/desktop/examples/browser_workspace_smoke.rs` 目前是 Windows 条件实现；
   Mac 的空入口/unsupported/退出 0 **不能算通过**。复用 `examples/fixtures/` 的语义场景，接入真实 Mac 执行分支，
   记录动作、trusted 事件、默认行为、最终状态以及非零失败退出。包括框架 iframe/Shadow DOM、Retina/多屏、
   中文 IME、上传/下载、popup/dialog、Stop/取消、切换会话、隐藏/显示、关闭退出与数据隔离。
5. **真实 Agent**：用本机已经配置且用户授权的 `step-3.7-flash`，走正式 Agent 配置与会话发送链路。
   在临时前端项目上启动 dev server，Agent 打开内嵌页面、操作控件、观察结果；需要修复时改临时项目并复测。
   不增加用户可见的“测试步骤/问题/控制台”产品。证明 Browser 工具实际送到模型且被调用，避免只测裸宿主。
   `examples/support/browser_agent_turn.rs` 是脚本 fixture；`browser_live_agent.rs` 是真实模型 fixture，证据分开记录。
6. **密钥**：`scripts/validation/run-nomi-core-live-provider-smoke.mjs` 说明了安全凭证输入与 `--browser` /
   `--browser-gui` / `--compile-only` 模式。Mac 先适配原生 fixture，再运行；不把 Windows PowerShell 取密钥脚本
   当跨平台入口。不得把 key 放进命令行、日志、prompt、Git 或 Cargo 构建环境；无本机凭证时标注 live 未执行。
7. **安装包**：先 `bun run build:mac arm --check`（Intel 改为 `intel`），再构建本机架构实际安装启动。
   `--check` 仅预检，不是构建或验收。具备 Developer ID 时再 `bun run build:mac --signed arm`；无签名凭证时
   可验证本地构建，明确区分本地/ad-hoc、Developer ID 与公证。Universal 能编译不等于双架构已实测。
8. **回传 Windows**：共享 Rust/renderer/合同/wry 改动必须在 Windows 重跑编译、定向测试和原生 smoke；
   Mac 端不能声称替 Windows 验收。Linux 本轮后置。

## 10. 验证状态与交付材料

- Windows 旧版原生验收发生在本次远程合并前，旧 EXE 不代表合并版。合并版编译、定向测试结果以本次 Git 交付说明为准。
- macOS 原生嵌入、可信输入、数据隔离、安装包、个人浏览器授权 attach 均需本机补证；Windows fixture 不能替代。
- 系统浏览器临时 profile/cookie fixture 与用户正在使用的已登录浏览器是不同证据，不得混记。
- 接手执行 prompt：[`2026-09-16-browser-workspace-v2-macos-work-prompt.zh.md`](2026-09-16-browser-workspace-v2-macos-work-prompt.zh.md)。
- Mac 返回：提交 SHA、环境版本、改动文件、通过/失败/未运行矩阵、去敏日志、产物路径与 SHA-256、
  仍需 Windows 回归项。不新增大型测试平台或长期 TODO；未实现能力保持 unavailable。
