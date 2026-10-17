# Native browser validation

This page records repeatable entry points, evidence boundaries and remaining native coverage.
The product contract lives in [Browser Workspace](../architecture/browser-platform.md).
Source and receipts from the current run determine the actual result.

## Choose the scope

| Change | Smallest relevant entry point |
| --- | --- |
| Shared browser state and layout | Affected `ui/` tests and `bun run check:browser-platform-boundary`; add `bun run check:desktop-ui-boundary` for renderer changes |
| Browser ownership from first use through Turn completion | Affected [Browser Role owner](../../crates/backend/nomifun-app/src/router/engine_browser_tools.rs), [coordinator](../../crates/backend/nomifun-browser-platform/src/run_guard.rs), HTTP and BrowserPanel tests; native/production GUI evidence remains separate |
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
- Verify browser ownership separately from Session status: Turns that never invoke the managed browser, including chat, file tasks, attached Chrome and background render, leave it available. The first real managed invocation locks it through subsequent tool completion, thinking, unrelated tools, pause and native settlement. Release requires the exact durable Turn terminal and successful finish; cancellation alone, unknown effects and uncertain cleanup do not unlock it. Test button/shortcut closure, exact-generation reopen, acquisition/close races and stale callbacks without replaying user commands.
- Test Cookie, localStorage and IndexedDB across close/reopen and process restart, Session/store isolation and completed native temporary-store cleanup. Do not substitute imported everyday-browser cookies.
- Compare sites on the same machine/network with controlled initial state. Record embedded identity and system-browser results separately, leaving each idle for at least 60 seconds. Distinguish normal, captcha, version rejection, pending and looping results; do not solve challenges or repeatedly refresh to obtain a pass.
- Validate the shared renderer at desktop sizes only, with an 880×600 minimum. Simulated component geometry does not establish native pixels, input or focus behavior.
- `CORE_PASS`, a single exit 0 or entry checks do not replace site completion, exact native exit, the controller's final receipt and resource cleanup evidence.

## Remaining coverage

As of October 15, 2026, historical targeted Windows runs cover navigation, Profile, popup
recovery, same-document History and the minimum desktop window. They do not establish all
platforms or every active-Agent interaction.

Acquisition on first browser use, ownership retained through the Turn and closing an unused browser during an active
Agent Turn still require native and production-GUI acceptance on both platforms. Historical input-lock records and
component tests do not establish this complete contract. Include tool completion followed by thinking or unrelated
work, pending navigation/dialog completion, failed settlement/unlock and exact-terminal recovery. Close or reopen
the browser before its first use, then invoke it in the same Turn; stale generations and old cleanup must not affect
its replacement.

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
