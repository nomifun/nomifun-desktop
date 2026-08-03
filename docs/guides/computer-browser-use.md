# Computer Use And Browser Use

NomiFun exposes two optional automation capability families to agents:

- **Computer use**: screenshots, mouse/keyboard input, window enumeration, and
  focus control through the in-process Rust implementation (`nomi-computer`,
  with accessibility helpers in `nomi-a11y`).
- **Browser use**: a single main-process `BrowserSessionHub` manages Chromium
  Hosts, addressable Browser Lanes, identity, resource queues, and cleanup.
  `nomi-browser-engine` supplies the CDP driver and
  `nomi-browser` supplies the Lane-aware tool adapter. Native Nomi, Gateway,
  ACP/Codex, remote agents, and parallel AgentExecution attempts all enter
  this Hub.

Both are high-privilege capabilities. In the desktop product UI they are
compiled in and enabled by default so a user can opt out from Settings. In
headless/server hosts they are omitted or disabled unless the host explicitly
enables the relevant build feature and runtime flag.

## Current Architecture

The old external `@playwright/mcp` sidecar and the private Native/Gateway/ACP
`BrowserTool` or Chromium ownership paths have been removed. The
`mcp-browser-stdio` bridge is now a scoped proxy to the Hub; it does not create
a browser or profile.

Computer use is desktop-oriented. It can observe the screen and synthesize
input, so it is compiled into desktop/Nomi CLI builds but omitted from the
headless web/server build.

## Enabling And Disabling Capabilities

### Desktop Settings

The desktop app exposes both toggles under System Settings:

- **Browser Use** (`/settings/browser-use`)
- **Computer Use** (`/settings/computer-use`)

Current desktop builds default both capability toggles to **on** when the
corresponding feature is compiled. Turning either toggle off persists a user
preference and prevents new sessions from receiving that capability. Browser
Settings also provides:

- browser source: a system Chrome/Edge executable or the managed source;
- presentation: the global default is **Silent** (`headless`), which runs
  ordinary Primary work with Chromium `--headless=new`. The installation owner
  may choose **Visible window** (`external`) here or on `/browser`; the backend
  applies a confirmed change immediately and persists it;
- resource policy: Automatic, Resource saving, or High concurrency;
- advanced resource limits for diagnostics and explicit tuning.

The sidebar **Browser** page (`/browser`) lists running and queued Lanes and
shows status, capacity, queue, identity, owner, and lifecycle data. It can close
a Lane, a conversation's Lanes, or all Lanes when authorized. For a running
Primary Lane it also offers **Open browser in foreground** and **Run in
background**. These change the current shared Primary Host without changing the
global default. The installation owner can edit that default live from this
page. The page still does not embed the page or provide page input, tab control,
user takeover, or address navigation.

### Per Session

Create or update a session with capability flags in `extra`:

```json
{ "computerUse": true, "browserUse": true }
```

Both camelCase and snake_case keys are accepted by compatibility paths.

### Host Environment

```bash
NOMIFUN_COMPUTER_USE=1
NOMIFUN_BROWSER_USE=1
```

These set default availability for Nomi-engine sessions in the host where they
are read. They do not bypass build-time feature gates.

### Nomi Engine Config

`~/.nomi/config.toml` or project `.nomi/config.toml`:

```toml
[tools]
max_recent_images = 3

[tools.computer]
enabled = true
max_screenshot_edge = 1568

[tools.browser]
enabled = true
# The trusted global default is headless; an installation owner may change it
# live to external. Foreground/background Lane actions do not rewrite it.
# Unknown legacy keys (e.g. old browser_path / idle_timeout_secs entries) are
# ignored and cannot bypass BrowserSessionHub policy.
```

On first use, the Hub resolves a system Chrome/Edge executable or the managed
source without requiring Node, npm, or Playwright. The selected source chooses
only the executable: every process remains managed by NomiFun and always uses
an application-owned isolated profile, never the user's real Chrome or Edge
profile. Two live Chromium processes are never allowed to open the same
user-data directory.

After a proven normal shutdown of a stable managed profile, NomiFun removes the
exact completed ownership marker and that launch's `DevToolsActivePort` file.
Cookies, site storage, and other stable profile data remain. If exact ownership
or process-tree exit cannot be revalidated, cleanup preserves those artifacts
for recovery and reports the shutdown as incomplete rather than guessing.

## Build Matrix

| Host | Computer use | Browser use |
| --- | --- | --- |
| `nomifun-desktop` | Compiled by the `computer-use` feature | Compiled by the `browser-use` feature |
| `nomi` CLI | Enabled in the current `nomi-cli` build | Not enabled in the current `nomi-cli` manifest |
| `nomifun-web` / Docker | Not compiled | Not compiled in the current headless web host |

Web/server builds should not promise desktop or managed-browser control. If a
config enables these tools in a host that was built without the relevant
features, the backend should warn rather than expose a non-working tool.

## Browser Lanes, Identity, And Concurrency

- A runtime keeps a stable default Lane for its lifetime. Parallel attempts in
  one AgentExecution receive different LaneKeys even when companion and
  conversation fields match.
- Operations within one Lane are strictly serialized. Different Lanes can run
  concurrently without crossing target, frame, ref, tab, download, or
  cancellation state.
- Ordinary interactive work defaults to the **Primary shared live identity**.
  The global presentation default is `headless`, so ordinary Agent use runs
  Chromium with `--headless=new` and creates no OS browser window. The
  installation owner may explicitly make `external` the live, persisted
  default. A Lane owner may also foreground or background only the current
  Primary Host without changing that default. Primary Lanes share cookies and
  profile-backed site state, but never share active targets, frame/ref cursors,
  operation gates, or downloads.
- Public reads default to **Anonymous crawl**, with no Primary cookies or site
  storage. Bounded read-only authenticated expansion may use an
  **Authenticated replica**; replica changes never merge back automatically,
  and identity-changing operations require Primary. Account switching,
  sign-out tests, untrusted browsing, and explicit isolation use an
  **Isolated identity**. Crawl, replica, and isolated Hosts may run headless.

Capacity is explicitly bounded. Requests beyond the safe budget enter a
cancellable queue and return `browser_capacity_queued` or
`system_memory_pressure` with queue position, reason, recommended concurrency,
and retry delay. `browser_open` successfully reports that its Lane entered the
queue, but navigation, observation, page `wait`, and other ordinary actions are
not dispatched until the Lane is `running`; they return an explicit retryable
tool error with `ok: false`, `dispatched: false`, and
`browser_capacity_queued` instead. After `retry_delay_ms`, call
`browser_status` and retry the original action only when it reports `running`.
Do not use page `wait` to wait for Lane capacity or launch another browser to
bypass it; reuse a running Lane, lower concurrency, or use
`browser_crawl_many` for bounded batch reads.

## Browser Tools And Lifecycle

Existing navigation, observation, action, screenshot, tab, download, and debug
actions accept an optional `lane_id`; omission uses the caller's default Lane.
Lane management actions are:

- `browser_open`: idempotently open the default or a named Lane;
- `browser_fork`: create an expansion Lane;
- `browser_list` / `browser_status`: inspect Lane, identity, capacity, queue,
  and recovery state;
- `browser_close` / `browser_close_all`: close one or all Lanes owned by the
  caller;
- `browser_crawl_many`: process a bounded URL batch while owning Lane reuse,
  ordering, cancellation, and cleanup.

Closing a Lane gives its browser calls a typed error but does not close the
conversation or AgentExecution. Attempt completion/cancellation, runtime
termination, conversation deletion, remote disconnect, capability expiry, and
app exit revoke owner leases and trigger authoritative drains. A Native Agent
turn drains its owner Lanes when it completes or is cancelled. A
session/conversation close drains matching Lanes and their target cleanup, then
shuts down any Host made empty. Installation shutdown and installation-wide
**Close All** prevent new opens while draining every Lane, pending cleanup, and
managed Host.

The UI confirms installation-wide **Close All** only when the response includes
all three authoritative zeroes: `remaining_lane_count`,
`remaining_cleanup_count`, and `remaining_managed_host_count`. A successful
request or a nonzero `closed` count alone is not proof that cleanup finished.

### Changing Primary Visibility

To inspect the real managed browser, open `/browser`, select a Lane whose
identity is Primary and whose state is `running`, then choose **Open browser in
foreground**. This is a one-time display request for the current Primary Host;
it does not change the global default. **Run in background** is the symmetric
current-Host action. Because Primary Lanes share one canonical Host, a required
headless/headful change safely replaces the whole Host at a new browser epoch
and rebinds every live Primary Lane on it. The same transition occurs when the
installation owner changes the global default and a running Primary Host has
the other mode. The Hub makes a best effort to restore active URLs, but old
target/frame/ref state is stale: refresh inventory and perform a fresh observe
before continuing. This does not change Lane ownership or add an embedded
preview, takeover, or page-input surface. Queued, failed, and non-Primary Lanes
cannot use these visibility actions.

The authenticated management endpoints are
`POST /api/browser/lanes/{id}/foreground` and
`POST /api/browser/lanes/{id}/background`; state-changing requests retain the
normal CSRF protection. They are not exposed as Agent Browser actions.
Installation owners manage the persistent default through
`GET`/`PUT /api/browser/display-mode`, as used by Settings and `/browser`.

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
- Browser actions derive approval from behavior: observation is info-level;
  navigation, clicking, typing, and other page mutations are execution-level.
  The Browser management page cannot invoke those actions. Egress, approval,
  secret, download, full-power, and irreversible-action safeguards remain in
  force, and model input cannot manufacture trusted approval.

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
