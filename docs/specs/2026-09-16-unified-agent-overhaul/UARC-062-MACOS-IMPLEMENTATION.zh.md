# UARC-062 macOS Process、PTY、Computer 与生命周期证据

## 1. 交付状态

UARC-062 的源码实现、真实 macOS Process/PTY gate、当前 TCC 状态和正式产品 Computer denied/read-only 链路
已经在 Apple Silicon 真机闭合；任务仍保持 `active`，不提前写成 `macOS verified`。以下需要用户在图形会话中
完成的原生矩阵仍为 `blocked`：

- 当前 NomiFun 的 Accessibility 已授权，但 Screen Recording 未授权；因此 granted screenshot、Retina 坐标和
  截图后的物理输入链路尚不能运行；
- 后续外部状态审计确认 macOS 图形会话已经 unlocked；真实 AppKit Command-Q、Computer launch/input、
  Command/Option/Control、Terminal UI focus/Unicode/resize 已通过。真实输入法 composition 仍未闭合：自动化的
  input-source shortcut 尝试只把 raw pinyin 送入 PTY，不能冒充中文 IME 成功。

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
- 同一 fixture 的 `--computer-input` 模式只操作 disposable TextEdit 文件：launch 后等待 owned filename 成为
  foreground Accessibility window，每次 input 后重新 observe 并传入新的 Role Host `expected_generation`；
  `cmd+right`、`option+left`、`ctrl+e` 与两次 type 最终生成 `alpha XbetaY`，`cmd+s` 后用最新 observation
  证明保存状态，再由 harness 关闭 TextEdit。
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
known_limitations: deterministic model proves product integration only; granted screenshot remains blocked
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

### 4.5 正式产品 Computer launch/input 与 modifiers

```text
task_id: UARC-062
scenario: signed product AgentSession -> Kernel -> Computer launch/input with fresh observation generations
steps: prepare a disposable TextEdit file; execute guarded computer/launch; update_plan; observe until the exact owned
       filename is foreground; set value; before every physical input call a11y.observe and pass the returned
       expected_generation; press cmd+right, option+left and ctrl+e with typed X/Y; verify alpha XbetaY; cmd+s;
       observe the saved state; report_completion; close TextEdit from the harness
expected: exact launch/input Actions succeed without stale authority; final text and file are alpha XbetaY;
          turn_completed + host_cleanup_proven; no TextEdit process remains
observed: model_calls=22; input_verified=true; all input observations usable at their exact workspace epoch;
          turn/completed; host_cleanup_proven=true; TextEdit AX value and saved file both alpha XbetaY;
          file sha256=84b9e548cc6792754abda7090c9f75538275f686e53fd0a59776eace8000eb19;
          harness TextEdit process count=0
result: pass
artifacts:
  build.noindex/uarc062-computer-input-evidence-v11/fixture-status.json
  build.noindex/uarc062-computer-input-evidence-v11/events.json
  build.noindex/uarc062-computer-input-evidence-v11/messages.json
  build.noindex/uarc062-computer-input-evidence-v11/file-result.json
  build.noindex/uarc062-computer-input-evidence-v11/stdout.log
known_limitations: no Screen Recording was used; Retina pixel fallback remains separately blocked
```

### 4.6 Terminal UI、Command-Q 与未闭合项

```text
task_id: UARC-062
scenario: real Terminal renderer focus, Unicode PTY round trip, resize and Command-Q cleanup
steps: use the signed product UI to create a $SHELL Terminal; focus its native Terminal input; paste and execute
       printf of 终端中文-日本語-✓; verify persisted raw PTY scrollback; enter full screen and return; invoke
       Command-Q with the PTY active; verify terminal rows/scrollback and process owner cleanup
expected: focus retained; exact UTF-8 output; PTY dimensions track UI; app quit deletes owned Terminal state/process
observed: focus=true; Unicode bytes present; dimensions 99x35 -> 191x48 -> 99x35; App quit;
          terminal cleanup deleted=1; host=0; terminal row=0; scrollback row=0; cleanup errors=0
result: pass for focus, Unicode, resize and Command-Q cleanup
artifact: build.noindex/uarc062-terminal-ui-evidence-v2/terminal-ui-result.json
known_limitations: native Unicode paste passed, but a synthetic input-source shortcut emitted raw pinyin; real IME
                   composition is not verified

task_id: UARC-062
scenario: Screen Recording granted + Retina screenshot/input
result: blocked
reason: Screen Recording=false；granted screenshot/Retina requires user grant + app relaunch

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
| packaged Computer launch/input/modifiers | `turn/completed` + `host_cleanup_proven`；22 model steps；saved `alpha XbetaY` |
| Terminal product UI | focus + UTF-8 round trip + 99x35→191x48→99x35 resize + active-PTY Command-Q passed；IME composition not verified |
| production UI + exact arm64 `.app` build | passed；7,677 modules；17m22s cold release build |
| exact helper + CEF staging + Developer ID seal | passed；host/framework/5 helpers arm64；deep strict valid |
| real AppKit Command-Q with active CEF | passed；host/helper=0；0 native/cleanup errors |

## 6. 未完成项

用户为当前 signed NomiFun 授予 Screen Recording 并重新启动后，才能运行 granted screenshot 与 Retina
coordinate matrix；真实中文/日文 IME composition 仍需用户在已配置的输入法下完成一次人工回合。上述 gate
闭合前，UARC-062 保持 `active / engineering verified / external gates blocked`；UARC-063 仍不得从 `planned`
提前升级，UARC-064 与 UARC-070 也不得标记完成。
