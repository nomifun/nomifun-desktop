# Native browser validation

This page records repeatable entry points, evidence boundaries and remaining native coverage.
The product contract lives in [Browser Workspace](../architecture/browser-platform.md).
Source and receipts from the current run determine the actual result.

## Choose the scope

| Change | Smallest relevant entry point |
| --- | --- |
| Shared browser state and layout | Affected `ui/` tests and `bun run check:browser-platform-boundary`; add `bun run check:desktop-ui-boundary` for renderer changes |
| Browser ownership during actual browser operations | Affected [Browser Role owner](../../crates/backend/nomifun-app/src/router/engine_browser_tools.rs), [coordinator](../../crates/backend/nomifun-browser-platform/src/run_guard.rs), native input adapters, HTTP and BrowserPanel tests; native/production GUI evidence remains separate |
| Rust browser domain and host integration | `bun run test:browser`, or select affected crates through the [Rust test runner](../../scripts/test-rust.mjs) |
| Windows native navigation, Profile, input and popups | [browser_workspace_smoke.rs](../../apps/desktop/examples/browser_workspace_smoke.rs) and its [support modules](../../apps/desktop/examples/support/) |
| macOS main thread and termination | [macos_appkit_main_loop_smoke.rs](../../apps/desktop/examples/macos_appkit_main_loop_smoke.rs), [macos_appkit_termination_probe.rs](../../apps/desktop/examples/macos_appkit_termination_probe.rs) |
| macOS WK navigation, permissions and sites | [macos_browser_compatibility_smoke.rs](../../apps/desktop/examples/macos_browser_compatibility_smoke.rs) |
| macOS fixed launch and final exit receipts | [Fixture packaging](../../scripts/validation/package-macos-native-fixture.mjs), [application controller](../../apps/desktop/examples/macos_application_controller.rs), [WK App acceptance](../../scripts/validation/run-macos-wk-app-acceptance.mjs) |
| Production window interaction | [browser-main-gui-fixture.mjs](../../scripts/validation/browser-main-gui-fixture.mjs) with the actual Tauri window |

Use each runner's `--help`, argument parser and source for platform prerequisites and options.
Use isolated data and record the source baseline, working changes, build identity, arguments,
exit status and cleanup receipts. Confirm the actual process, dynamic ports and window on every
run; old PIDs, temporary files and another build do not identify the current instance.

The background Chrome engine's pipe EOF, windowless startup, stale singleton locks and OOPIF
attachment require separate platform-specific validation; start at
[nomi-browser-engine](../../crates/agent/nomi-browser-engine/src/lib.rs). Native embedded-panel
acceptance does not establish the background engine's lifecycle or cleanup behavior.

## Evidence boundaries

- Report static checks, units, native smoke, production GUI, external sites and signed release validation separately. Building and signing do not prove native execution.
- Record exact target, navigation sequence, attempt and document identities alongside server main-document request counts. Separate URL changes, SPA transitions, subresources and redirects from repeated main-document requests.
- Exercise Stop, Pause/Resume, queued/claimed popups, late callbacks, retained content after failure, and same-document/cross-document Back/Forward through their native lifecycle. Resume must not replay old candidates or navigation.
- Verify browser ownership separately from Session status: actual managed browser work acquires the gate; native completion, pressed-input cleanup and applicable Effect result recording precede local release. While the same Turn continues thinking or using other tools, the user can browse and close the runtime. A pending dialog or retained native work keeps the gate until safe completion; cancellation or uncertain cleanup does not prove release. Test button/shortcut closure, exact-generation reopen, acquisition/close races, dropped callers and stale callbacks without replaying user commands. Cover observe → release → act under a new guard so safe cleanup does not invalidate all observations; navigation, closed tabs and replaced runtime generations must still reject stale references.
- Test Cookie, localStorage and IndexedDB across close/reopen and process restart, Session/store isolation and completed native temporary-store cleanup. Do not substitute imported everyday-browser cookies.
- Compare sites on the same machine/network with controlled initial state. Record embedded identity and system-browser results separately, leaving each idle for at least 60 seconds. Distinguish normal, captcha, version rejection, pending and looping results; do not solve challenges or repeatedly refresh to obtain a pass.
- Validate the shared renderer at desktop sizes only, with an 880×600 minimum. Simulated component geometry does not establish native pixels, input or focus behavior.
- `CORE_PASS`, a single exit 0 or entry checks do not replace site completion, exact native exit, the controller's final receipt and resource cleanup evidence.

## Remaining coverage

As of October 15, 2026, historical targeted Windows runs cover navigation, Profile, popup
recovery, same-document History and the minimum desktop window. They do not establish all
platforms or every active-Agent interaction.

On October 18, 2026, the isolated Windows WebView2 `browser_workspace_smoke --interaction-only`
run passed with exit 0 after `cargo build --profile test -p nomifun-desktop --example browser_workspace_smoke`.
Its disposable localhost fixture covers observe → release → act under a new guard with the previous reference,
one trusted native click, native user-policy restoration and user commands between calls. Pending chained dialogs
retain their original callback and block user close/destruction until the final response unlocks input. User close
destroys the native controller and its usable protocol endpoint; a read-only OS handle observes the exact browser
main process alive before close and exited within the subsequent five-second deadline. The same Session can then
create a new runtime generation. Native fixture profile cleanup also passed. The process witness retains no COM
environment, drops test view handles before waiting and never kills a browser process.

Production-GUI acceptance on both platforms and macOS native acceptance of this handoff remain outstanding.
The Windows fixture does not establish installed-package behavior, external-site compatibility or every child
process's exit. Complete production coverage must include browser completion followed by thinking or unrelated
tools, manual input and runtime closure after release, pending navigation/dialog completion, failed
settlement/unlock and exact-terminal recovery. Recheck observe → release → act, then user navigation or closure
followed by another browser call in the same Turn. Safe cleanup must preserve usable observations, while stale
generations and old cleanup cannot affect a replacement. Record backend/unit, actual native input and
production-window evidence separately for the current source baseline.

The latest macOS popup Pause, same-document History and launch-receipt fixes have source,
targeted checks and build records. Complete native WK and production-GUI evidence for the
combined source remains outstanding:

1. Run the current complete static UI App and both smoke examples; verify the fixed descriptor, LaunchServices admission, build identity, native exit and final cleanup receipts.
2. Pause/Resume popups at deterministic queued/claimed stages; verify exact child closure, active tabs, parent-document nonce and main-document request counts.
3. Exercise Stop, failure recovery, stale controls/callbacks, input locks, and same-document/cross-document Back/Forward natively.
4. Verify persistent-store restart, Session isolation and temporary-store cleanup.
5. Operate the production address bar, tabs, menus, pause, visibility and recovery; capture safe navigation timelines and screenshots.
6. Compare Safari17, NativeWebKit and system Safari independently. The original Baidu repeated-navigation report still lacks completed reproduction, attribution or proof of repair.

An Apple Silicon development build does not establish minimum-OS, Intel, Developer ID or
notarized-release acceptance. Record tool restrictions accurately; cases that cannot launch
remain unrun without weakening launch, permission or cleanup contracts.

## Historical records

Dated implementation reports, handoffs, per-attempt logs and screenshots are retired from the
working tree and remain in Git history. Find a pre-deletion commit with
`git log --all -- docs/reports docs/handoffs`, then read a file with
`git show <commit>:<path>`. Historical evidence covers only its stated baseline and scope;
it is neither acceptance for new source nor a current development requirement.
