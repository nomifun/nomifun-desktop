# UARC-064 macOS 合入后的 Windows 回归记录

> macOS closeout：`5fd6c6f0643206283aec17be83b8de8d5853f10e`
> Windows 回归源码：`2a426e7af3367ba47101fa3e676a71c89bc57af6`
> 平台：Windows x64 verified；macOS 证据保持冻结，不由本任务重解释

## 结论

macOS CEF、Computer 与打包改动合入后，Windows WebView2、Browser Resource/Action、Process、PTY、Computer、
Desktop、UI、商业模型与 NSIS 候选均通过回归。UARC-064 completion gate 已满足：Mac fixes 没有回归 Windows
共享行为。

审计中发现 macOS human-only external Browser/Downloads handoff 使用 `open::that_detached`，其语义合法但未登记在
Process boundary 的 exact reviewed allowlist。`2a426e7af` 将且仅将
`apps/desktop/src/browser_surface/macos/native/external_browser.rs` 加入 allowlist，并新增 scanner self-test；任意
其他路径仍 fail closed。该边界负责用户明确的 OS handoff，不把 Agent Process authority 移出统一 owner。

## Rust 与原生回归

```text
native Browser stability                         8 passed
nomifun-browser-platform                         56 passed
nomifun-gateway                                  107 + production bypass audit passed
nomifun-ai-agent (browser-use)                   449 passed, 1 ignored
nomifun-app (browser-use)                        488 passed
nomifun-agent-domain-wave2                       18 passed
nomi-computer                                    103 passed, 7 explicit real-device ignores
nomi-computer safe real read-only cases          6 passed
nomi-process-runtime                             217 passed
nomifun-terminal                                 135 passed
nomifun-desktop                                  144 passed, 3 installed-Chrome/network ignores
```

真实 Computer 测试覆盖 screenshot、两个 cursor position、primary/invalid display capture 和 window listing；
会移动用户鼠标的 `mouse_move_real` 继续按安全策略不运行。Process 回归包含 Windows Job、ConPTY、parent-death、
timeout/cancel、descendant cleanup、PTY stdin/resize/UTF-8 和终态冻结。

真实 child-WebView2 默认矩阵通过 trusted click/Unicode input、nested wheel、pointer capture drag、input lock、
hide/resize/show 与 profile cleanup。统一 `--agent-only` 场景继续完成 15 次 scripted model call、exact
navigate/observe/act/upload/download、adaptive plan/completion、trusted input、download publication、target_blank
cleanup、canonical history 和 active-resource shutdown。

## 商业模型

Windows Credential Manager 隔离的 StepFun Coding Plan `step-3.7-flash` 在当前源码上通过：

```text
live_smoke_compile_status=pass code=OK status=200
live_smoke_phase=execute mode=selected_model model=step-3.7-flash
live_smoke_status=pass code=OK status=200
browser_live_frontend_status=pass native_click=true observed_value=2 canonical_action_shape=true evidence_cancel=true terminal_unlock=true
```

凭据未进入 argv、Cargo/build-script 环境、仓库、fixture 或日志；gate 结束后环境变量不存在。

## UI 与边界

- UI：3,538 passed / 0 failed。
- Production renderer：7,561 modules，build passed；只有既有 chunk size/dynamic import 提示。
- `bun run check`：typecheck、1,916-source 880×600 desktop boundary、i18n、theme、icons、dead CSS、Windows
  installer、Creative Studio retirement、Process/Browser/UARC boundary、Agent vocabulary 和 help 全部通过。
- UARC scanner：所有 retired production families 为 0；既有 AutoWork baseline anomaly 为 1；macOS gaps 为 0。
- rustfmt 与 `git diff --check` 通过。

## Windows 当前源码候选

clean HEAD `2a426e7af3367ba47101fa3e676a71c89bc57af6`：

```text
installer: NomiFun_0.7.6_x64-setup.exe
size:      64,094,527 bytes
sha256:    8f7b0fdc3d33b8cbd69f78de6cdbd2be2ec0c9e89ce6fcf06eb341801ca83a57

host:      220,742,656 bytes, PE 0x8664
host sha:  e5a724d474197de4fc2fb40f4218a6335f74b78398713aa49c257cb8cfa0f63e
```

candidate harness 14/14 通过：native host、clean source checkpoint、work-root、installer、安装前置、静默隔离安装、
installed binary、启动、port announcement、`/health` 200、WebView2 CDP、完整进程树 cleanup、卸载和残留验证。
应用退出后 0 个进程、0 个候选安装目录；原开发 `nomifun://` 注册树经备份后原样恢复。

首次 release build 在 `nomifun-gateway` rustc/LTO 阶段发生 Windows `STATUS_ACCESS_VIOLATION`；同一 clean
commit、相同 build 命令的精确重跑越过该点并完成 release、NSIS 与 candidate smoke，因此记录为已复现的
Windows rustc toolchain 瞬态，不豁免任何产品 gate。

## 平台状态

- Windows：verified at `2a426e7af`。
- macOS：UARC-061/062/063 的 verified evidence 保持在 Mac closeout；本任务不把其显式 `not_run` 项改写为 pass。
- 下一步：UARC-070 requirements-to-evidence、production reachability、write/delete/retained ownership 和临时代码
  最终审计。
