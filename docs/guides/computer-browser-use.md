# Computer Use And Browser Workspace

NomiFun separates desktop-wide Computer automation from a conversation's native browser. Both require the corresponding Agent capabilities; a page or model prompt cannot grant those permissions.

## Browser Workspace (v2, implementation in progress)

Open the browser from the Browser button inside a conversation. The global Browser management/settings page and its compatibility redirect have been removed.

The user and Agent see the same native web page. During an Agent run, user page input is locked. Stop the Agent and wait for pending operations to settle before interacting manually. Hiding the panel keeps the page and tabs alive; it is not a takeover or a reset.

For a frontend project, start its development server, then ask the Agent to navigate to the localhost URL in the conversation browser, observe the page, interact, and verify the result. Navigation, clicks, keyboard input and standard HTML selections use the real browser. A screenshot stream is not the interactive surface.

Windows native input has real smoke coverage. The full product acceptance matrix, including macOS, frames, dialogs, files, and packaging, is still in progress. See the [architecture](../architecture/browser-platform.md) for the current limits.

## Optional Attached Chrome (Windows, implementation in progress)

An Agent session's `browser` capability can be served by an attached,
already-running Chrome instead of the embedded Browser Workspace. The provider
is installation-level: connect once with
`POST /api/browser-providers/attached-chrome` after enabling Chrome remote
debugging (`chrome://inspect/#remote-debugging`), approve Chrome's prompt, then
bind the session to the `attached-chrome` provider resource. `GET`/`DELETE` on
the same route read and release the attachment.

No Chrome restart, replacement profile, or import of cookies, passwords or
history is required. Disconnect releases automation and its connection, not
your browser or tabs. Mutable browser operations fail closed while the Agent
owns the run.

The frontend wiring and the full iframe/file/dialog acceptance matrix remain
in progress; the provider and its REST surface are implemented in
`nomifun-app` (`browser_workspace_provider/attached_provider.rs`). Edge, macOS
and Linux support is not yet claimed. This capability grants neither
embedded-browser control nor local or provider-native web search.

## Optional Web Research

The `web.research` capability module covers public-web research independently
of the model provider's native search. Queries run through an isolated
background browser runtime and do not use session login state; enabling it does
not grant Browser automation.

Windows desktop detects installed Chrome 120+; without a suitable installation
this capability cannot be enabled. Discovery does not launch a browser. A query
starts an isolated runtime and verifies its live version.
Queries go to Bing and engine domains are resolved through Google Public DNS
over HTTPS, without session login state. Restart the app if a Chrome update
invalidates the pinned release.

## Computer Use

Computer automation remains desktop-oriented and separate from Browser Workspace. Select it in the Agent workbench, then use Settings → Capabilities & Permissions → Computer Use to inspect and grant OS access. Before creating a session with Computer actions, the product checks the exact frozen action set and links here when a required grant is missing, but it does not block session creation or the Agent's other capabilities; the affected action returns a recoverable permission error only when invoked. A `computer/launch` action needs neither Screen Recording nor Accessibility.

An enabled Computer card in Agent Workbench names the exact missing OS grants and links to the permission center; this is a runtime-readiness reminder, never authority persisted into the Agent. If an existing conversation has never received—or later loses—a required grant, its header auto-opens a permission bubble once. Dismissing the bubble leaves a warning chip available, and returning from System Settings triggers a live re-check.

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
not to the terminal/editor. Accessibility is re-probed when the app regains
focus. A newly granted Screen Recording permission can remain cached for the
running process, so completely quit and reopen NomiFun before treating it as
still denied. Permission-failure messages name "NomiFun" explicitly so the
guidance is unambiguous.

Settings → Capabilities & Permissions → Computer Use surfaces a live status panel (macOS): it shows whether
Accessibility / Screen Recording are *in effect for the running process* —
which is authoritative, since a System Settings toggle bound to a stale
code-signing identity reads "Not in effect" even while it looks on — with
buttons that deep-link to the exact Privacy pane and trigger the OS prompt.
Backed by `GET/POST /api/system/permissions[/request|/open-settings]`
(`nomi_computer::permissions` → `AXIsProcessTrusted` /
`CG*ScreenCaptureAccess`).

## Voice, Browser, Notification, and File Permissions

- **Voice input (ASR)** requests the microphone only after the user presses the voice or test button. The test immediately releases the stream and neither stores nor uploads audio. A denial links directly to the Voice Input permission tab.
- **Browser Use** navigation, page observation, visible-page screenshots, clicking, and typing do not borrow Computer Use Accessibility or Screen Recording grants. macOS 14+ uses system WKWebView; Agent clicks and form fills use semantic page actions, without trusted user activation. Dragging, hover, key combinations, embedded frames and automatic file transfer are unavailable in this version. See the [capability matrix](../architecture/browser-platform.md). Site/device permissions remain explicit user decisions; system prompts may also apply. If the concrete Browser provider is unavailable, the Agent session still starts with only that tool surface omitted.
- **Local Network** access is requested on macOS 15 and later when the Browser opens a LAN address or NomiFun actively connects to a local SSH/MCP service or companion device. Merely listening for inbound WebUI TCP connections needs no grant. The App carries the usage description.
- **System notifications** require both the in-product preference and the OS grant. Enabling normal or scheduled-task notifications requests the native grant and provides a recovery link if it was denied.
- **Files and protected folders** use a workspace, upload, or file-picker result selected by the user. NomiFun does not ask everyone for Full Disk Access; that remains an optional recovery choice only after an intentional protected-folder operation is denied.
- **macOS website file selection** does not prefilter file types from HTML `accept`, because the public WK file-panel API does not supply those MIME/extension hints. Choose a file type accepted by the website. Windows keeps its existing file filters.
- **Robot device permissions** such as camera, motion, display, and proactive speech remain owned by Device and Companion settings and are rechecked against the Agent's exact actions when its tool surface is resolved and before dispatch. An offline device omits only Robot tools; it does not block the Agent's other capabilities. These are not host macOS permissions, and grants on this page never replace them.

Permission reads require the installation owner. Requests and System Settings deep-links additionally require the desktop process's local-trust proof, so a remote WebUI session cannot pop privacy UI on the host. The old `/api/computer/permissions*` routes remain compatibility aliases only.

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
