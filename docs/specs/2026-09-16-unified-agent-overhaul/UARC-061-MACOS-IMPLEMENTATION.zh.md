# UARC-061 macOS Browser 实现与原生证据

## 1. 交付状态

UARC-061 主实现已在真实 Apple Silicon Mac 上完成生产接线和工程验证，但任务仍保持 `active`，不提前写成
`macOS verified`。以下三个外部步骤尚未闭合：

- Keychain service `NomiFun/StepFun/LiveProvider` 不存在，真实 `step-3.7-flash` gate 为 `blocked`；
- 当前 macOS 会话被锁定，Computer Use 明确要求用户手动解锁，因此 Command-Q 事件路径为 `blocked`；
- Google Chrome 已安装，但默认 profile 没有 `DevToolsActivePort`，因此真实产品连接前置条件不成立；需要用户在
  `chrome://inspect/#remote-debugging` 显式开启 Remote Debugging 后才能执行安装级 attached Chrome gate。

这些 blocker 没有被 mock、免费模型、SIGTERM 或独立 fixture 冒充为通过。

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
StepFun Keychain credential: absent
Developer ID Application identity: available
notarization credential: available；submission/stapling owned by UARC-063
```

## 4. 原生证据

### 4.1 CEF native surface

```text
task_id: UARC-061
scenario: signed native CEF child surface and Browser runtime conformance
steps: build fixture + helper; stage pinned framework and five helpers; Developer ID sign; launch real AppKit/Tauri
       window; execute input/frame/upload/picker/download/popup/dialog/permission/storage matrix; shutdown CEF
expected: every check true, arm64 helpers, shutdown_complete=true, no owned process remains
observed: 33/33 checks true; shutdown_complete=true; user and Agent downloads use real CEF callbacks
result: pass
artifacts:
  build.noindex/uarc061-cef-final/run-AemOgK/native-result.json
  build.noindex/uarc061-cef-final/run-AemOgK/artifact.json
  build.noindex/uarc061-cef-final/run-AemOgK/stdout.log
  build.noindex/uarc061-cef-final/run-AemOgK/stderr.log
known_limitations: fixture receipt is explicitly productAcceptance=false; product evidence is separate below
```

覆盖项包括真实 trusted click、Unicode 输入、frame/OOPIF 几何、upload、AppKit file picker hide-cancel、
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
host sha256: a15507b5593d0620819e37fbf553d422688553a7b07d4225e1a8edbedc6eee56
framework sha256: ff6bc94f3be51d647c1cfc19a2467e2beaf000b4557a304c012b040662558890
main helper sha256: a4345cae5f32826e3cf05fe319f2350c3a26400289c72800c075b535187573d1
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
result: blocked
reason: macOS is locked and Computer Use requires manual unlock; verified SIGTERM cleanup is retained only as
        additional lifecycle evidence and is not relabeled Command-Q

task_id: UARC-061
scenario: live StepFun Coding Plan step-3.7-flash Browser turn
result: blocked
reason: Keychain service NomiFun/StepFun/LiveProvider is absent; no alternative model was used
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
| final signed CEF native smoke | passed；33/33 + shutdown |
| packaged product Agent/Kernel/CEF smoke | passed；terminal + cleanup receipt |

## 6. 未完成项

UARC-061 只有在用户安全录入 Keychain commercial credential、解锁 Mac 完成 Command-Q、并显式开启 Chrome
Remote Debugging 后，才能把三项 blocker 重新运行并将任务从 `active` 更新为 `integrated / macOS verified`。
UARC-064 与 UARC-070 仍不得提前标记完成。
