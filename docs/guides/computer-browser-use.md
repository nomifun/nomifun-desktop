# Computer Use And Browser Workspace

NomiFun separates desktop-wide Computer automation from a conversation's native browser. Both require the corresponding Agent capabilities; a page or model prompt cannot grant those permissions.

## Browser Workspace (v2, implementation in progress)

Open the browser from the Browser button inside a conversation. The global Browser management/settings page and its compatibility redirect have been removed.

The user and Agent see the same native web page. During an Agent run, user page input is locked. Stop the Agent and wait for pending operations to settle before interacting manually. Hiding the panel keeps the page and tabs alive; it is not a takeover or a reset.

For a frontend project, start its development server, then ask the Agent to navigate to the localhost URL in the conversation browser, observe the page, interact, and verify the result. Navigation, clicks, keyboard input and standard HTML selections use the real browser. A screenshot stream is not the interactive surface.

Windows native input has real smoke coverage. The full product acceptance matrix, including macOS, frames, dialogs, files, and packaging, is still in progress. See the [architecture](../architecture/browser-platform.md) and [implementation record](../specs/2026-09-13-browser-workspace-v2-progress.zh.md) for the current limits.

## Optional System Browser (Windows, implementation in progress)

Select `nomi_system_browser` in the Agent workbench's Web category, then use System Browser in the conversation header to connect to your running, signed-in Chrome 144+.
Enable connections at `chrome://inspect/#remote-debugging`, approve Chrome's prompt, and choose the tabs this conversation may use.
No Chrome restart, replacement profile, or import of cookies, passwords or history is required. Chrome's native permission covers the selected profile; NomiFun separately restricts Agent targets to your selected tabs.

The initial driver supports main-document observation, navigation, clicks, text, keys and scrolling in the original Chrome window. There is no screenshot-stream surface, test panel or takeover mode.
Authorization changes are disabled while the Agent runs. NomiFun cannot physically lock Chrome's address bar, window controls or permission-revocation UI.
Disconnect releases automation and its connection, not your browser or tabs. A pending connection can be cancelled; uncertain requests are reconciled by reading state, never automatically reconnected or replayed.

Trusted main-document input has been tested in an owned disposable Chrome. User-approved personal-login acceptance, the full iframe/file/dialog matrix, and release acceptance remain outstanding.
Edge, macOS and Linux support is not yet claimed. This capability grants neither embedded-browser control nor local or provider-native web search.

## Optional Web Search

In the Agent workbench's Web category, `nomi_local_websearch` is distinct from the model provider's `web.search`. Local search sends the query to a search engine through an isolated Headless runtime and does not use conversation login state. It does not require model-native search or grant browser automation.

Windows desktop detects installed Chrome 120+; without a suitable installation this capability cannot be enabled. Discovery does not launch a browser. A query starts an isolated runtime and verifies its live version.
Queries go to Bing and engine domains are resolved through Google Public DNS over HTTPS, without conversation login state. Restart the app if a Chrome update invalidates the pinned release.
Windows catalog integration and public queries are verified. Full main-application interaction and packaging acceptance remain in progress; macOS implementation awaits the later handoff.

## Computer Use

Computer automation remains desktop-oriented and separate from Browser Workspace. Use the Agent workbench to select its capabilities and Settings → Computer Use to manage desktop-control settings and OS permissions.

Standalone Nomi Computer configuration remains:

```toml
[tools]
max_recent_images = 3
[tools.computer]
enabled = true
max_screenshot_edge = 1568
```

## macOS Permissions

Computer use needs OS permissions the first time it is used:

- **Accessibility**: required for mouse/keyboard input and accessibility tree
  operations.
- **Screen Recording**: required for screenshots. A black screenshot usually
  means this permission is missing.

These run **in-process inside the desktop app**, so the permission must be
granted to **NomiFun itself** (the entry named "NomiFun" in System Settings),
not to the terminal/editor — and a freshly-granted permission only takes effect
after the app is **completely quit and reopened** (macOS does not hot-load TCC
grants into a running process). Permission-failure messages name "NomiFun"
explicitly so the guidance is unambiguous.

Settings → Computer Use surfaces a live status panel (macOS): it shows whether
Accessibility / Screen Recording are *in effect for the running process* —
which is authoritative, since a System Settings toggle bound to a stale
code-signing identity reads "Not in effect" even while it looks on — with
buttons that deep-link to the exact Privacy pane and trigger the OS prompt.
Backed by `GET/POST /api/computer/permissions[/request|/open-settings]`
(`nomi_computer::permissions` → `AXIsProcessTrusted` /
`CG*ScreenCaptureAccess`).

> **Stale grant.** If a toggle is clearly on yet computer use still fails, the
> grant is bound to an older build's identity. Quit NomiFun, run
> `tccutil reset Accessibility com.nomifun.desktop` and
> `tccutil reset ScreenCapture com.nomifun.desktop`, relaunch, re-grant, and
> fully restart once more.

## Approval Semantics

- Read-only computer actions such as `screenshot`, `cursor_position`,
  `list_windows`, and `wait` are treated as info-level operations.
- Mutating computer actions such as click, type, scroll, drag, and
  `focus_window` are execution-level operations and require approval in default
  modes.
- Plan mode hides the whole computer-use tool.
- Browser observation is info-level; navigation and page changes are execution-level. The run guard and exact capability policy apply independently of page content.

Recommended loop: observe with a screenshot or browser snapshot, perform one
small operation, then observe again.

## Image And Token Hygiene

- Screenshots are downsampled to a maximum long edge of
  `max_screenshot_edge` pixels, with coordinates mapped back to real screen
  coordinates. The final PNG is also capped at 5 MiB; high-entropy frames are
  downscaled again and coordinate geometry follows the exact image sent.
- The conversation keeps only the most recent `max_recent_images` individual
  tool-result images, with a provider-compatible ceiling of 20 per request and
  a cumulative encoded-payload budget. Excess attachments are stripped while
  their text and an omission note remain. Provider errors also remove replayed
  screenshots before the conversation is persisted for recovery.
- OpenAI-compatible tool messages cannot carry images directly; image data is
  sent as a following user message with a source call id. Anthropic, Bedrock,
  and Vertex use native image blocks where supported.
- External MCP image results pass through the same image pipeline with a
  per-image size cap.

## Related Docs

- [Agent Engine](../architecture/agent-engine.md)
- [MCP And Skills](mcp-and-skills.md)
- [Remote Capability API](remote-capability-api.md)
