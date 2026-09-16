# 可直接交给 macOS 开发 Agent 的工作 prompt

你在 macOS 电脑上接手 NomiFun 的 Browser Workspace v2。请实现并自行验证 macOS 原生内嵌浏览器，
保持 Windows 已有能力，不扩张产品范围。请先阅读仓库 AGENTS.md，以及
`docs/continuity/2026-09-16-browser-workspace-v2-macos-handoff.zh.md` 全文，再检查实际代码。
文档里的 Windows 证据不是 macOS 已完成的证据。

## 代码交接

仓库分支是 `rf/agent-capability-platform-v2`。Windows Browser 保全提交 `476b6bd0b` 已与远程
`a8b1685cf` 的多 Engine 等改动整合；以远程分支最新、包含此 prompt 的提交开工。
先检查当前分支和 `git status`，保护已有未提交改动。工作区干净时 fetch origin；已有跟踪分支用
`git pull --ff-only origin rf/agent-capability-platform-v2` 更新，没有本地分支则从远程建立跟踪分支。
出现本地分叉不要 reset、force push 或悄悄丢提交，先说明分叉并安全合并。
如需独立 Mac 开发分支，使用 `codex/browser-workspace-macos`；记录基线 SHA。
不要 checkout 到旧保全提交，不要转而合入 main。
不要从 Windows 复制数据库、浏览器 profile、登录态、API key、target、node_modules 或 EXE。

## 产品边界（必须遵守）

- 页面必须是会话内真实原生 WKWebView，用户看到的就是 Agent 操作的同一页面。
- 禁止 iframe、JPEG/PNG 连续帧、screencast、Canvas 远程桌面、另一个 headless 页面替身。
  单次截图可用于 Agent 观察/验收，但绝不是浏览器的呈现方式。
- Agent 工作时不允许用户操作浏览器；只有 Agent 已停止且原子操作 settle 后才能恢复用户输入。
  不存在“接管/交还”产品，也不能靠全局抢鼠标妨碍用户使用其他应用。
- Agent 模拟用户点击、输入、滚动、观察、测试前端只是底层能力；沿用现有简单 Browser UI。
  不做测试步骤、问题面板、终端/控制台面板、DevTools/F12、弹出独立浏览器、Browser 设置中心。
- 交互浏览器沿用系统网络/代理/证书，支持 localhost、LAN、WebSocket/HMR；只保留最小安全边界，
  不建设网络代理平台或大规模 allowlist。网页不能访问 Tauri 特权/应用密钥。
- `nomi_local_websearch` 是工作台可选的独立能力，后台 Search/Render 不能占用可见页或借用个人登录态。
  不恢复旧 headless 用户浏览器产品，不因已有后台实现而将它用作原生页面降级。
- `nomi_system_browser` 是另一套能力：操作用户正在使用、已经登录的系统浏览器，attach-only，不导入数据。
  本轮优先交付 Mac 内嵌浏览器；系统浏览器未有本机可靠授权与原生操作证据之前保持 unavailable，
  单独报告接入缺口，不以临时新 Chrome/profile 充数。
- 无数据库迁移，不兼容已废弃的 Browser Hub/Fresh-v4 host。Windows wry close 补丁不能删除。
- 只支持桌面最小 880x600，不添加手机/平板适配。Linux 后置。

## 执行方法

1. 先记录环境与未改动基线编译结果，检查 handoff 点位表，列出最短实现路径。
2. 先做最小原生闭环：主窗口 child WKWebView + 观察 + 可信 click/type/press/wheel + 输入锁 + Stop。
   用真实页面证明 `event.isTrusted`、默认行为、焦点和中文输入。WKWebView 不能假定有 CDP；
   不用 DOM click/dispatchEvent/直接改 value 冒充用户输入。核心动作如受公开 API 限制，提供具体证据并明确阻塞，
   不通过新增第二套控制平台绕过。
3. 复用现有 BrowserWorkspace/Tab/RunGuard/Role/renderer slot，平台差异收敛到最小原生适配。
   补齐 Retina 坐标、resize、遮挡、hide/show、会话切换、数据隔离、上传下载、dialog/popup、关闭退出。
   页面不应因缩放布局或隐藏显示而丢失状态。保留新合入的 macOS 退出和 PTY 清理逻辑。
4. 走正式配置与会话执行链验证 Browser 声明和绑定，尤其检查 Role defaults 重载不丢 Browser。
   当前 Nomi Engine 有 Browser 接入；不要误称 Coding Engine 自动具备同能力。
5. 自己执行 handoff 的定向 Rust/UI/原生 fixture 检查；UI 改动跑 desktop boundary。
   Cargo 串行以免构建锁干扰；独立 UI 检查可并行。原生 fixture 不能因 cfg 排除/零测试/unsupported 退出 0 而算通过。
6. 使用本机已配置、用户授权的 `step-3.7-flash` 进行真实前端 Agent 闭环；不要要求用户反复手工代测。
   凭证不存在时仅报告真实模型项未执行，不复制 Windows 密钥，不泄漏密钥给构建脚本或日志。
7. 构建本机 Mac 安装包并实际启动测试，记录架构和签名级别；没有 Developer ID 就明确公证未验证。
8. 删除此次适配过程中产生的废弃实现和测试替身，不做无关重构；完成后提交代码并给出 Windows 回归清单。

## 返回结果

以可审计的简洁报告交付：基线/最终提交、代码点位、macOS/架构/工具版本、
定向测试与真实 WKWebView fixture 的通过/失败/未运行项、真实模型与脚本 fixture 的证据区分、
包路径与 SHA-256、已知不支持项、需要回 Windows 验证的具体命令/场景。
未验证就说明未验证；不能仅凭编译通过、截图正常或临时 Chrome 测试宣称全量完成。
