# UARC-062 macOS Process、PTY、Computer 与生命周期证据

## 1. 交付状态

UARC-062 的源码实现、真实 macOS Process/PTY gate、当前 TCC 状态和正式产品 Computer denied/read-only 链路
已经在 Apple Silicon 真机闭合；任务仍保持 `active`，不提前写成 `macOS verified`。以下需要用户在图形会话中
完成的原生矩阵仍为 `blocked`：

- 当前 NomiFun 的 Accessibility 已授权，但 Screen Recording 未授权；因此 granted screenshot、Retina 坐标和
  截图后的物理输入链路尚不能运行；
- 后续外部状态审计确认 macOS 图形会话已经 unlocked；真实 AppKit Command-Q 已在 active CEF Session 上通过。
  Command/Option/Control、Computer input/launch 和 Terminal UI focus/IME 继续作为独立物理矩阵闭合，不能由
  backend tests 替代。

这些 blocker 没有被 mock 输入、合成截图或错误地把 TCC 拒绝当成 Tool 成功来绕过。

## 2. 实现摘要

- 修复 macOS native API 从无 AppKit/CFRunLoop 的 test/CLI host 调用时，`dispatch2::run_on_main` 可无限等待的
  问题。若调用者已在 main thread，任务立即执行；否则任务异步排入 main queue，并以一秒 admission 窗口判断
  event loop 是否可用。
- admission 超时会先从共享 slot 撤销尚未开始的 closure，再返回 `macOS main event loop is unavailable`，避免
  caller 已观察失败后输入或 launch 迟到执行；若 main queue 已领取任务，则等待其精确结果，保持物理 effect
  outcome 不被遗弃。
- 扩展 disposable `browser_gui_fixture`，让正式 `.app` 通过 canonical AgentSession、Compiler、Kernel、
  Computer Resource/Action owner 执行 `computer/a11y.observe` 与 `computer/observe`。fixture 校验 model-safe
  `ROLE_HOST_PROVIDER_FAILURE`，并按正式 Engine 的 `update_plan`、requirements、`report_completion` 协议收口，
  不重试被 TCC 拒绝的 screenshot。
- 现有 Process/PTY 实现无需平台分叉：真实 macOS suites 已证明 process group/generation、timeout/cancel、
  descendant/parent-death cleanup、Seatbelt roots、PTY stdin/resize/UTF-8/fast output 和 Terminal shutdown。

## 3. 环境

```text
source base: a39e2bfee807ec1fe1cea0cea957663a66c03558
branch: rf/agent-capability-platform-v2
macOS: 26.3 (25D125)
hardware: Mac mini Mac16,10 / Apple M4 / 32 GB / arm64
Xcode: 26.5 (17F42)
rustc: 1.96.0
cargo: 1.96.0
bun: 1.3.14
node: v22.19.0
Tauri CLI: 2.11.2
TCC: Accessibility=true；Screen Recording=false
StepFun Keychain credential: absent
Developer ID Application identity: available
notarization credential: available；submission/stapling owned by UARC-063
```

## 4. 原生证据

### 4.1 Process runtime 与 parent-death

```text
task_id: UARC-062
scenario: real macOS process group/generation, cleanup and Seatbelt contracts
steps: cargo test -p nomi-process-runtime -- --test-threads=1
expected: exact group identity; timeout/cancel/descendant cleanup; abrupt parent exit cleanup; truthful Lost;
          Unicode argv/env/cwd; scoped Seatbelt roots; no leaked process/session authority
observed: 240 passed / 0 failed; both real parent-death tests removed process and PTY groups;
          cancel/kill removed same-group grandchildren; setsid escape remained Lost
result: pass
log: build.noindex/uarc062-validation/nomi-process-runtime.log
known_limitations: real Command-Q is recorded separately below
```

### 4.2 PTY 与 Terminal backend

```text
task_id: UARC-062
scenario: PTY stdin, resize, UTF-8 locale, fast output and terminal shutdown
steps: cargo test -p nomifun-terminal -- --test-threads=1
expected: real PTY round trips, Unicode locale, quick/fast output retention, resize and exact process-tree cleanup
observed: 146 passed / 0 failed; included real PTY child locale/Unicode filename, input, process-group kill,
          shutdown cleanup and abrupt backend-exit leader/grandchild cleanup
result: pass
log: build.noindex/uarc062-validation/nomifun-terminal.log
known_limitations: renderer focus and IME remain a separate UI gate
```

### 4.3 Computer TCC 状态与正式产品链路

```text
task_id: UARC-062
scenario: signed product AgentSession -> Kernel -> local Computer owner, granted Accessibility + denied Screen Recording
steps: build exact arm64 app/helper; stage pinned CEF; Developer ID sign; prepare disposable Computer-enabled
       AgentSession and deterministic loopback model; launch exact app; query /api/computer/permissions; execute exact
       a11y.observe then screenshot Actions; consume canonical denial; update_plan/report_completion; verify terminal and
       cleanup; terminate through verified desktop shutdown
expected: Accessibility observation succeeds; screenshot is a recorded Tool error with
          ROLE_HOST_PROVIDER_FAILURE; no retry; turn_completed and host_cleanup_proven; no app/helper remains
observed: Accessibility=true, Screen Recording=false; a11y_observed=true; screen_denied=true; model_calls=5;
          exact actions computer/a11y.observe + computer/observe; turn/completed; host_cleanup_proven=true;
          product/helper process count after shutdown=0
result: pass for granted Accessibility and denied Screen Recording paths
artifacts:
  build.noindex/uarc062-computer-denied-evidence-v6/permissions.json
  build.noindex/uarc062-computer-denied-evidence-v6/fixture-status.json
  build.noindex/uarc062-computer-denied-evidence-v6/events.json
  build.noindex/uarc062-computer-denied-evidence-v6/messages.json
  build.noindex/uarc062-computer-denied-evidence-v6/stdout.log
  build.noindex/uarc062-computer-denied-evidence-v6/stderr.log
  build.noindex/uarc062-computer-denied-evidence-v6/shutdown.txt
known_limitations: deterministic model proves product integration only; physical input and granted screenshot remain blocked
```

### 4.4 exact-source arm64 app

```text
task_id: UARC-062
scenario: current-source arm64 app/helper rebuild and Developer ID seal
expected: current Computer dispatcher in arm64 host; pinned CEF framework and five helpers; deep strict signature valid
observed: production renderer 7,677 modules; optimized app build passed; helper build passed; host/framework/helper arm64;
          codesign --verify --deep --strict passed; TeamIdentifier D3TDA9B335; hardened runtime enabled
result: pass
artifact: target/aarch64-apple-darwin/release/bundle/macos/NomiFun.app
host sha256: bf16ff136b621cc37cc556fb7875afdbf7306f41f3f0cdb06b495f2ff5173feb
framework sha256: 7b26c48d2ba2809b5c652cb7761859e6fd5f66c67bbeb8dfd236d0d35548210c
main helper sha256: 7e55783f97ed14ca004b814d75d4f3d1a1ecc9d88f89077b8f769e17e75e6faa
logs:
  build.noindex/uarc062-validation/tauri-arm64-app-build.log
  build.noindex/uarc062-validation/cef-helper-build.log
  build.noindex/uarc062-validation/cef-stage.log
  build.noindex/uarc062-validation/artifact.txt
known_limitations: DMG/release lock/notarization remain UARC-063 gates
```

### 4.5 granted Computer、Terminal UI 与 Command-Q

```text
task_id: UARC-062
scenario: Screen Recording granted + Retina screenshot/input + modifiers + Terminal focus/IME
result: blocked
reason: Screen Recording=false；granted screenshot/Retina requires user grant + app relaunch；the now-unlocked
        modifier/input/Terminal focus/IME subset continues independently

task_id: UARC-062
scenario: Command-Q process-tree cleanup
steps: complete a canonical Agent/Kernel/CEF turn with five live helpers; use Computer Use to send Command-Q;
       verify App quit, owner shutdown logs and process tree
observed: App quit; host=0; helpers=0; cleanup_error_count=0; native_error_count=0;
          ChannelMessageLoop and Terminal cleanup stopped normally
result: pass
artifact: build.noindex/uarc061-command-q-evidence-v5/command-q-result.json
```

## 5. 工程 gate

| Gate | 结果 |
| --- | --- |
| `cargo test -p nomi-process-runtime -- --test-threads=1` | 240 passed |
| `cargo test -p nomifun-terminal -- --test-threads=1` | 146 passed |
| `cargo test -p nomi-computer -- --test-threads=1` | 92 passed；7 个真实物理/TCC tests explicit ignored |
| `cargo fmt --all -- --check` | passed |
| Computer permission API | Accessibility=true；Screen Recording=false |
| packaged Computer Agent/Kernel smoke | `turn/completed` + `host_cleanup_proven`；canonical denial，无重试 |
| production UI + exact arm64 `.app` build | passed；7,677 modules；17m22s cold release build |
| exact helper + CEF staging + Developer ID seal | passed；host/framework/5 helpers arm64；deep strict valid |
| real AppKit Command-Q with active CEF | passed；host/helper=0；0 native/cleanup errors |

## 6. 未完成项

用户为当前 signed NomiFun 授予 Screen Recording 并重新启动后，才能运行 granted screenshot 与 Retina
coordinate matrix。Command/Option/Control、Computer launch/input 和 Terminal focus/IME 在当前 unlocked Session
继续闭合。上述 gate 闭合前，UARC-062 保持 `active / engineering verified / external gates blocked`；UARC-063
仍不得从 `planned` 提前升级，UARC-064 与 UARC-070 也不得标记完成。
