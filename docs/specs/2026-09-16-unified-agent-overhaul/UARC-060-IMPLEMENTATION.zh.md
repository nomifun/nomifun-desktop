# UARC-060 Windows 集成验收记录

> 输入 barrier：`d14af94e611de1cef986287f72cb027cf770e4ef`
> 实现提交：`b09c680bbedd939a3dd9c5a05d9432a6ae79bdee`
> 平台：Windows x64 verified；macOS pending，真机闭环由 UARC-061/062/063 完成

## 交付

- 桌面与 Web 启动统一通过 canonical startup data root 进入新 Agent Store，不再从历史 Conversation 或旧
  Runtime 入口绕行。
- 新增统一 Runtime 的 Browser module owner。每次 Action 都校验 principal、Session、Turn、Snapshot、
  Provider、Resource 与 Action grant，并冻结 `BrowserSessionAuthority`；managed Browser 延迟绑定到当前 run，
  attached Chrome 只通过 provider-private adapter 进入共享 Browser Resource/Action 合同。
- Browser `navigate`、`observe`、`act`、dialog、upload、download、evaluate 与 render content 接入官方 Wave 2
  target、通用 compiler、Kernel、effect ledger、取消与生命周期收口。未知效果按真实执行阶段分类，终态必须在
  解锁前写入。
- PlatformBuiltin compiler 按当前 Snapshot 中已选择、已激活的 FunctionTool actions 通用生成工具；名称为稳定
  provider-safe hash，长度不超过 64，schema 保持 exact，不再维护 Nomi/Coding/Browser 手写分支。
- `EngineSessionHost` 规范化并校验 host workspace，再作为 `AdmittedEngineSession` 的只读 authority 传入；
  Browser owner 与其他 owner 共享同一 Session/Turn 边界。
- canonical `browser/act` 输入改为扁平 action，并直接接受 `browser/observe` 返回的完整 element。owner 丢弃
  display metadata，只信任冻结的 generational reference；upload/download 使用同一 element 形状，陈旧引用
  fail closed。
- `browser_workspace_smoke --agent-only` 现在覆盖 AgentSession → generic compiler → Kernel → Browser owner →
  WebView2 的生产链；live fixture 使用 canonical AgentSession API、显式 managed Browser resource 和认证 WebSocket。
- Windows Browser 继续使用 WebView2；没有引入 WKWebView，也没有改变 macOS 独立 CEF child NSView 的固定架构。

## 删除与保留边界

- 物理删除 `browser_agent_downloads.rs` 与 `browser_tool_roundtrip.rs`；其覆盖迁入统一生产 smoke，不保留平行
  Browser Runtime 或仅服务历史架构的 wrapper。
- 从 UARC-054 closeout 到实现提交共修改 37 个文件，新增 2,810 行、删除 1,469 行。
- 保留可维护的 Windows candidate harness、WebView2 native fixture、attached Chrome adapter、共享 Browser
  Resource/Action contract 和 macOS CEF 底层实现；macOS CEF 尚未因此获得生产/打包完成声明。

## 自动化验证

最终源码上的结果：

```text
nomifun-agent-domain-wave2                 18 passed
nomifun-app (browser-use)                 488 passed
nomifun-desktop                           144 passed, 3 ignored
nomifun-browser-platform                  55 passed
nomifun-gateway                           107 unit + 1 production-bypass audit passed
nomifun-ai-agent (Browser feature)        449 passed, 1 ignored
Windows native stability Bun suite        8 passed
Computer real read-only suites            6 passed
bun test --cwd ui                         3538 passed
bun run check                             passed
workspace Rust all-targets + doctests     passed on final rerun
cargo fmt --all -- --check                passed
git diff --check                          passed
```

`bun run check` 包含 1,914 个 renderer source 的 880×600 desktop boundary、typecheck、i18n、theme、icons、
dead CSS、Windows installer contract、Creative Studio retirement、Process/Browser/UARC boundary、vocabulary 与
help 检查。UARC scanner 的 retired production families 均为 0；既有 AutoWork baseline anomaly 保持 1，
macOS gaps 保持 pending 3，没有用 Windows 结果消除它们。

完整 UI 生产构建通过。workspace Rust 首次全量运行曾由 `rustc` 在第三方 `aws-sdk-bedrock` 编译期间发生
`STATUS_ACCESS_VIOLATION`；相同源码、相同命令的精确重跑通过所有 crate 与 doctest，因此记录为 Windows
toolchain 瞬态而非产品通过项豁免。

## Browser 与商业模型证据

确定性 WebView2 smoke 在最终 action schema 上通过默认 native 场景和 `--agent-only` 场景。统一 Agent 场景
完成 15 次模型调用，覆盖 navigate、observe、可信 click、upload、download、adaptive plan/completion、精确
下载 bytes/hash、`target_blank` 清理、同页终态、canonical history 与 active resource shutdown。

Windows Credential Manager 中隔离的 StepFun Coding Plan `step-3.7-flash` 通过真实模型选择与 Browser gate：

```text
live_smoke_compile_status=pass code=OK status=200
live_smoke_phase=execute mode=selected_model model=step-3.7-flash
live_smoke_mode=selected_model model=step-3.7-flash
live_smoke_status=pass code=OK status=200
browser_live_frontend_status=pass native_click=true observed_value=2 canonical_action_shape=true evidence_cancel=true terminal_unlock=true
live_smoke_status=pass code=OK status=200
```

凭据只从 Windows Credential Manager target `NomiFun/StepFun/LiveProvider` 读取；未进入 argv、Cargo/build
script 环境、仓库、fixture 文件或日志。真实模型负责 navigate/observe/可信 click；见证成立后由 harness 通过
canonical turn cancellation 验证 owner cleanup/unlock。完整 upload/download 等链路由独立确定性 native smoke
覆盖，未伪装成模型自由生成的额外证据。

## Windows x64 候选制品

从 clean HEAD `b09c680bbedd939a3dd9c5a05d9432a6ae79bdee` 执行：

```text
bun run build:win x64
bun scripts/validation/run-windows-desktop-candidate-smoke.mjs \
  --installer dist/desktop/NomiFun_0.7.6_x64-setup.exe \
  --source-commit b09c680bbedd939a3dd9c5a05d9432a6ae79bdee \
  --work-root build.noindex/uarc060-windows-candidate-b09c680b
```

制品与安装后 host：

```text
installer: NomiFun_0.7.6_x64-setup.exe
size:      64,089,439 bytes
sha256:    ab572e619fb85b511488e25a94c76cf62f4ba7eace4c13751c43b1a6b6bb6dda
host:      220,760,064 bytes, PE 0x8664
host sha:  3508109b5b2fc876ee0af5c6f7899abebe04af16d4a8decb8615a5f88436273b
```

candidate harness 的 14 项检查全部通过：native host、clean source checkpoint、work-root、installer、安装前置、
静默隔离安装、installed binary、启动、port announcement、`/health` 200、WebView2 CDP（`NomiFun`，
`http://tauri.localhost/#/guid`）、含 6 个 descendant 的 process-tree cleanup、卸载和残留验证。

首次预检仅发现指向本仓库 `target/debug/nomifun-desktop.exe` 的既有开发协议注册；没有产品卸载项、安装目录
或运行进程。验收前将该精确注册树导出到 `build.noindex`，测试期间临时隔离，并在同一命令的 `finally` 中
原样恢复。候选未覆盖用户安装，结束后注册内容、安装目录和进程状态均已核对。

## 平台状态

- Windows：verified。共享合同、Store/Session/Runtime/Compiler、WebView2 Browser、Computer/Process、真实商业
  模型、UI、release build 与 NSIS install/uninstall 已形成同一 clean-source 候选闭环。
- macOS：pending。UARC-061/062 已满足依赖但必须在外部 Mac 真机实施；UARC-063 必须补齐 CEF child NSView、
  TCC、Retina/IME、进程生命周期、arm64 `.app`/DMG 与签名结构证据。未经这些证据不得声称跨平台完成。
