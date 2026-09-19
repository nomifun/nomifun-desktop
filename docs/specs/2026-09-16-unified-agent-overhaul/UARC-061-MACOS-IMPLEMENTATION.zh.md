# UARC-061 macOS Browser 实现与原生证据

## 1. 交付状态

UARC-061 已在真实 Apple Silicon Mac 上完成生产接线和核心原生验证，状态为
`integrated / macOS verified`。真实 `step-3.7-flash` 已通过隔离 stdin 执行 managed CEF/Workspace 功能链路；
Keychain 持久化与 attached Chrome Remote Debugging 按用户产品决定记为 `not_run`，不删除已有能力，也不使用
mock、免费模型、SIGTERM 或独立 fixture 冒充通过。

## 2. 实现摘要

- 将现有独立 CEF child `NSView` 作为 `BrowserRuntimeFactory` 接入生产 `main.rs`、
  `BrowserResourceService`、AgentSession Browser owner 和 App shutdown；所有 Runtime 先关闭，随后才执行
  process-wide `cef_shutdown`。
- 生产 bundle 只从 `NomiFun.app/Contents/Frameworks` 解析 pinned CEF framework 与 `NomiFun Helper.app`，
  不从 PATH、开发构建目录或 WKWebView 回退。
- 完成 tab、真实 `target_blank` popup、dialog、permission、用户/Agent 输入锁、logical/Retina bounds、
  hide/show/resize/close、诊断投影、用户文件选择、upload、用户 download/cancel、Agent download 私有落盘/
  校验/发布以及 storage clear。
- CEF download 正确处理 `OnDownloadUpdated` 可早于 `OnBeforeDownload` 的原生顺序；首个 native item 绑定
  一次显式 Agent click，其他 item 立即取消，terminal proof 后才发布到授权 workspace。
- 修复真实 Command-Q 暴露的 file-chooser lifecycle race：长驻用户 chooser 的三秒无事件窗口现在表示 idle，
  不再提前把 worker poison 为 `ActionInterrupted`；Agent upload 和 native conformance 仍保留严格三秒 event gate。
  若 AppKit 已先销毁 page protocol，chooser close 只在该可证明 terminal 状态吸收 listener error。
- attached Chrome 的 macOS discovery 使用 Chrome 默认 user-data root 下的 `DevToolsActivePort`，只连接，
  不启动、不接管 profile、不静默扩权。
- `browser_gui_fixture` 已适配 UARC-060 的 hashed `platform__*` exact Action Tool、engine-owned
  `update_plan`/requirements/`report_completion` 流程；fixture 只准备 disposable dataset 和本地 deterministic
  model/page，CEF、Browser service、Kernel、Resource、NSView 和 teardown 全由正式 `.app` 所有。

## 3. 环境

```text
source base: a8dbaae766fab4f1d99680701659ee3df03c4696
branch: rf/agent-capability-platform-v2
macOS: 26.3 (25D125)
hardware: Mac mini Mac16,10 / Apple M4 / 32 GB / arm64
Xcode: 26.5 (17F42)
rustc: 1.96.0
cargo: 1.96.0
bun: 1.3.14
node: v22.19.0
Tauri CLI: 2.11.2
CEF crate: 152.3.0+152.0.6
CEF archive: cef_binary_152.0.6+g708dc14+chromium-152.0.7977.83_macosarm64_minimal.tar.bz2
Chromium: 152.0.7977.83
StepFun credential: user-provided / isolated stdin；Keychain persistence not required
Developer ID Application identity: available
notarization: completed by UARC-063
```

## 4. 原生证据

### 4.1 CEF native surface

```text
task_id: UARC-061
scenario: signed native CEF child surface and Browser runtime conformance
steps: build fixture + helper; stage pinned framework and five helpers; Developer ID sign; launch real AppKit/Tauri
       window; execute input/frame/upload/picker/download/popup/dialog/permission/storage matrix; shutdown CEF
expected: every check true, arm64 helpers, shutdown_complete=true, no owned process remains
observed: 34/34 checks true, including native_renderer_crash_projection; shutdown_complete=true;
          user and Agent downloads use real CEF callbacks
result: pass
artifacts:
  build.noindex/uarc061-cef-final/run-AemOgK/native-result.json
  build.noindex/uarc061-cef-final/run-AemOgK/artifact.json
  build.noindex/uarc061-cef-final/run-AemOgK/stdout.log
  build.noindex/uarc061-cef-final/run-AemOgK/stderr.log
  build.noindex/uarc061-cef-commandq-fix-v2/run-oAyKhT/native-result.json
  build.noindex/uarc061-cef-commandq-fix-v2/run-oAyKhT/artifact.json
  build.noindex/uarc063-cef-crash/run-A0upQe/native-result.json
  build.noindex/uarc063-cef-crash/run-A0upQe/artifact.json
known_limitations: fixture receipt is explicitly productAcceptance=false; product evidence is separate below
```

覆盖项包括真实 trusted click、Unicode 输入、renderer crash→Crashed/input lock/channel close、frame/OOPIF 几何、upload、AppKit file picker hide-cancel、
用户 download/cancel、Agent private download publication、permission deny/Agent fail-closed、真实 popup opener/profile、
dialog drain、conversation storage isolation/clear 和 CEF shutdown。

### 4.2 正式产品链路

```text
task_id: UARC-061
scenario: packaged AgentSession -> compiler -> Kernel -> Browser owner -> CEF child NSView
steps: stage pinned runtime into release NomiFun.app; sign nested code and app with Developer ID; prepare disposable
       Browser-enabled canonical AgentSession; launch exact app; submit one turn; allow guarded first effect;
       update_plan with source-anchored requirement; run exact hashed navigate/observe/act Tools; report_completion;
       release deterministic model; verify terminal and cleanup; terminate through verified desktop shutdown
expected: exact Browser effects succeed; native page records Unicode text and trusted click; turn_completed and
          host_cleanup_proven; app/helper process tree exits
observed: final DOM contained "Agent 主界面真实输入" and "已收到真实点击"; witness count=1/trusted=true;
          navigate and act effects recorded succeeded; turn_completed=true; turn_failed=false;
          host_cleanup_proven=true; post-shutdown product/helper process query empty
result: pass
artifacts:
  build.noindex/uarc061-product-gui-evidence-v6/fixture-status.json
  build.noindex/uarc061-product-gui-evidence-v6/events.json
  build.noindex/uarc061-product-gui-evidence-v6/messages.json
  build.noindex/uarc061-product-gui-evidence-v6/stdout.log
  build.noindex/uarc061-product-gui-evidence-v6/stderr.log
known_limitations: deterministic local model proves integration only; it is not the required live StepFun gate
```

### 4.3 产品 bundle 与签名结构

```text
task_id: UARC-061
scenario: production arm64 app CEF bundle layout and Developer ID seal
expected: arm64 host/framework/helpers; exact pinned metadata and credits; deep strict signature valid
observed: host/framework/five helpers arm64; codesign --verify --deep --strict passed;
          TeamIdentifier D3TDA9B335; hardened runtime enabled; runtime.json and CREDITS.html present
result: pass
artifact: target/aarch64-apple-darwin/release/bundle/macos/NomiFun.app
host sha256: bf16ff136b621cc37cc556fb7875afdbf7306f41f3f0cdb06b495f2ff5173feb
framework sha256: 7b26c48d2ba2809b5c652cb7761859e6fd5f66c67bbeb8dfd236d0d35548210c
main helper sha256: 7e55783f97ed14ca004b814d75d4f3d1a1ecc9d88f89077b8f769e17e75e6faa
known_limitations: DMG/release lock/notarization are UARC-063 gates, not claimed here
```

### 4.4 Attached Chrome

```text
task_id: UARC-061
scenario: installed Chrome discovery and simple connection readiness
observed: Google Chrome 150.0.7871.114 installed; documented default
          ~/Library/Application Support/Google/Chrome/DevToolsActivePort absent; targeted adapter tests pass and the
          live product connection is not run without the user-enabled endpoint
result: blocked
reason: Chrome 144+ requires the user to enable Remote Debugging explicitly in chrome://inspect/#remote-debugging
```

### 4.5 Command-Q 与真实模型

```text
task_id: UARC-061
scenario: Command-Q cleanup
steps: launch the exact signed app with a completed canonical Agent/Kernel/CEF turn and five live CEF helpers;
       bind the NomiFun AppKit window through Computer Use; press Command-Q; verify UI termination, logs and process tree
expected: app quits through the real menu accelerator; no host/helper remains; no Browser/native cleanup error
observed: Computer Use returned App quit; host=0, helpers=0 immediately after; cleanup_error_count=0;
          native_error_count=0; ChannelMessageLoop and Terminal cleanup both stopped normally
result: pass
artifacts:
  build.noindex/uarc061-command-q-evidence-v5/command-q-result.json
  build.noindex/uarc061-command-q-evidence-v5/fixture-status.json
  build.noindex/uarc061-command-q-evidence-v5/events.json
  build.noindex/uarc061-command-q-evidence-v5/messages.json
  build.noindex/uarc061-command-q-evidence-v5/stdout.log

task_id: UARC-061
scenario: live StepFun Coding Plan step-3.7-flash Browser turn
steps: pass the user-provided credential only through the isolated fixture stdin; launch the exact signed product;
       navigate/observe/click the managed CEF page; reproduce count=2; patch workspace app.js; reload and click again
observed: real_provider=true; trusted witnesses=[2,1]; changed_source_served=true; fixture failure=null;
          host_cleanup_proven=true; credential artifact scan found zero matches
result: pass for the native Browser/Workspace workflow and cleanup
known_limitation: after functional completion, the model retried report_completion until compaction reached
                  MaxOutputTokens; this turn is not claimed as turn/completed
artifacts:
  build.noindex/uarc-live-model-scope/browser-live-product-v5/fixture-status.json
  build.noindex/uarc-live-model-scope/browser-live-product-v5/events.json
  build.noindex/uarc-live-model-scope/browser-live-product-v5/cleanup.json
```

## 5. 工程 gate

| Gate | 结果 |
| --- | --- |
| `cargo test -p nomifun-browser-macos --lib` | 11 passed |
| `cargo test -p nomifun-browser-platform` | 55 passed；doctest 0 failed |
| `cargo test -p nomi-browser-engine attached_browser --lib` | 49 passed；3 explicit real-Chrome ignores |
| `cargo check -p nomifun-desktop --no-default-features` | passed |
| `bun run check:browser-platform-boundary` | passed |
| `bun run check:uarc-boundary` | passed；baseline anomaly 1 / macOS gaps 3 不增长 |
| `bun run check:desktop-ui-boundary` | passed；1,916 renderer sources / minimum 880×600 |
| `bun run build:mac --check arm` | passed |
| production UI + arm64 release `.app` build | passed；7,677 modules |
| final signed CEF native smoke | passed；34/34 + renderer crash projection + shutdown |
| packaged product Agent/Kernel/CEF smoke | passed；terminal + cleanup receipt |
| real AppKit Command-Q with active CEF | passed；host/helper=0；0 native/cleanup errors |

## 6. 完成状态与 `not_run`

UARC-061 已按 managed CEF 核心范围更新为 `integrated / macOS verified`。用户明确决定 Keychain 持久化和
attached Chrome Remote Debugging 不属于本轮必要门槛：credential 通过隔离 stdin 使用，attached Chrome
实连记为 `not_run`；不删除现有 attached provider 能力。UARC-064 与 UARC-070 仍不得提前标记完成。
