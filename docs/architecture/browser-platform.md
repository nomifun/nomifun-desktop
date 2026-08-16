# Browser Platform Architecture

NomiFun's browser platform is an Agent-oriented managed Chromium service. Its
execution and scheduling unit is a **Browser Lane**, not an Agent session or a
Chromium process. The `/browser` page remains a status and lifecycle management
surface. Within that boundary it can manage the installation-wide Primary
display default and change the visibility of an existing running Primary Lane,
but it is not a browser rendering or page-control surface.

> **Superseding product decision (2026-07-27):** the former embedded preview,
> JPEG/screencast transport, dedicated viewer WebSocket, viewer token, and user
> takeover/return-control flow have been removed. References to those features
> in historical plans, requirements, or test records are superseded and are not
> current capabilities or work to restore. New installations use a global
> `headless` default, so ordinary Primary work launches Chromium with
> `--headless=new`. The installation owner may change that default live to
> `external`, while an explicit Lane foreground action is a one-time visibility
> change and does not rewrite the default. Neither path restores any former
> viewer capability.

## Core model

The application process constructs exactly one `BrowserSessionHub`. The Hub is
the sole authority for:

- managed Chromium Hosts and explicit shutdown;
- Lane ownership, lifecycle, queueing, and cancellation;
- Primary, Anonymous, Authenticated Replica, and Isolated identity routing;
- owner leases and trusted caller capabilities;
- global resource policy, dynamic telemetry, and periodic cleanup; and
- user-scoped inventory and realtime lifecycle events consumed by the Browser
  management page.

A `BrowserHost` owns one managed Chromium process tree and one internal CDP
connection. A Host may carry multiple Lanes. Each Lane has its own target/tab
set, active target/frame, ref generation, operation gate, cancellation token,
and download ownership.

Semantic operations within one Lane are strictly serialized. Different Lanes
may run concurrently. Global operation permits bound resource consumption; they
do not replace per-Lane serialization.

## Identity modes and browser presentation

| Mode | Purpose | Storage and presentation |
|---|---|---|
| `primary` | Interactive browsing, sign-in, and account-affecting work | Uses a stable, application-isolated profile. The global default is `headless`, which launches Chromium with `--headless=new`; an installation owner may explicitly select the live, persisted `external` default instead. A user may also foreground or background the current Primary Host without changing that default. Primary Lanes share live identity state. |
| `anonymous` | Public-page and knowledge-source crawl work | Uses a temporary isolated profile, never reads Primary cookies or site storage, and may run headless. |
| `authenticated_replica` | Bounded read-only authenticated crawl expansion | Uses a generation-tagged point-in-time isolated replica, never writes back automatically to Primary, and may run headless. |
| `isolated` | Account switching, logout tests, untrusted browsing, or explicit isolation | Uses an independent temporary identity and may run headless. |

Replica operations classified as possibly changing identity or persistent
account state fail with `needs_primary_identity`. Trusted action classification
happens in-process; model input cannot weaken it or fabricate user approval.

The Chromium executable may come from system Chrome/Edge or a managed source;
that choice selects only the binary. NomiFun always starts and owns a separate
managed process with its own application-isolated profile. It never opens,
copies, or takes ownership of a user's personal Chrome/Edge profile or process.

CDP and profile material are private implementation details. The renderer,
public or management APIs, Agent tools, realtime events, errors, and logs must
not expose raw CDP endpoints, debugging ports, profile paths, or profile
contents.

## Call paths and ownership

All production call paths ultimately use the same Hub:

- Native Nomi receives a main-process-issued `BrowserLaneClient` when its
  runtime is created.
- Gateway resolves `CallerIdentity` from authenticated conversation/runtime
  context before forwarding to the Hub.
- Feature-gated browser stdio bridges hold only a short-lived, renewable
  loopback capability scoped to an audience and operation set; they do not own
  Chromium.
- Each knowledge URL render uses a transaction-scoped Anonymous Lane. The
  renderer serializes `navigate` plus `rendered_html`, then closes that exact
  Lane on success, error, timeout, or cancellation; it does not pin a page
  between fetches.
- Browser sign-in uses a managed Primary Lane and follows the current trusted
  Primary display default; the user may still make a one-time foreground or
  background request from the management page.
- HTTP management endpoints invoke only user-scoped inventory and lifecycle
  boundaries—including changing the current visibility of an existing running
  Primary Lane—or installation-owner display and resource controls.

The model may select only a length-bounded Lane name. `user_id`, conversation,
runtime instance, attempt, owner lease, allowed operations, and expiry all come
from trusted application context.

When a runtime, attempt, remote connection, or capability ends, its owner lease
must be revoked and that owner's Lanes drained. Native Agent turn completion or
cancellation uses the same owner-scoped drain. Session/conversation closure
drains the matching Lanes, waits for their target cleanup, and shuts down Hosts
that become empty. Installation shutdown and installation-wide **Close All**
block new opens while draining every Lane, pending cleanup/retirement, and
managed Host before reporting completion. Normal cleanup does not rely on idle
expiry, a periodic sweep, or a warm timer.

Delayed transaction cleanup carries the exact Lane id and its sealed owner,
runtime, and task-family authority. Retry, dispatcher saturation, or cleanup
budget pressure never widens that authority into a later task scan or an
installation-wide `close_all`; it only reconciles already-retained exact debt.
This prevents an old cancelled crawl/fetch from closing a replacement Lane
created under the same runtime (the ABA case). Broad cleanup remains available
only through explicit owner/session/installation lifecycle operations that
fence new ingress before draining.

## Agent tools

Existing Browser actions may omit `lane_id`, in which case the caller's
`default` Lane is used. The platform also provides:

- `browser_open` to idempotently open a default or named Lane;
- `browser_fork` to create an expansion Lane;
- `browser_list` / `browser_status` to read Lane, identity, capacity, and
  recovery state;
- `browser_close` / `browser_close_all` to close the current owner's Lanes; and
- `browser_crawl_many` to crawl a URL set with bounded concurrency while
  owning Lane creation, reuse, cancellation, and cleanup.

`browser_open` may successfully return a structured `queued` Lane rather than a
"ready" handle blocked behind a hidden global lock. Queue metadata includes
position, suggested concurrency, owner/global active and queued counts, retry
delay, and a stable reason code. Until that Lane is `running`, every ordinary
page action—including page `wait`—returns an explicit tool error with
`ok: false`, `dispatched: false`, stable code `browser_capacity_queued`, and
retry metadata. `browser_status` remains the successful polling operation.

## Resources and lifecycle

The Automatic policy derives safe limits from total system memory and logical
CPU count. At runtime, the Hub samples available memory, CPU pressure, and
managed Chromium RSS and computes `normal`, `pressured`, or `critical` state.
Aggregate Browser memory is intentionally elastic across independent tasks: the
machine-wide ratio is a pressure threshold, not a fixed installation quota.
Automatic uses 40% of physical memory, Resource saving uses 30%, and High
concurrency uses 50%; the derived system reserve is capped at 8 GiB and global
operation/Lane capacity remains hardware-adaptive. This lets many concurrent
tasks use more than 1 GiB in total without allowing one task to monopolize the
installation.

Each trusted user-visible task family has its own envelope. Sibling runtimes in
one conversation share that family; rotating a runtime does not mint another
quota. Automatic and High concurrency both use a 1 GiB attributed-memory
budget, 2 weighted active operations, 4 open Lanes, and 16 top-level tabs;
Resource saving uses 768 MiB, 1 operation, 2 Lanes, and 8 tabs. High concurrency
raises only aggregate throughput and does not relax those per-task limits.
Operation, Lane, tab, queue, and internal state bounds are exact. Runtime and
owner-lease keys remain separate lifecycle authorities, so dropping one
runtime never closes a sibling in its family. A shared Chromium Host has only one operating-system
process tree, so its RSS cannot be measured exactly per task; the Hub attributes
that RSS across live task Lanes and uses the result as a reclaim watchdog. The
management API and Browser page therefore label the global value as a pressure
threshold and the per-task memory value as an estimate.

Remote MCP transport state follows the same bounded model. `/mcp` and
`/mcp-agent` share one machine-adaptive admission authority; request bodies,
session identifiers, scopes, provisional sessions, initialization rate, and
pending Browser cleanup debt are all bounded. Headerless non-`initialize`
requests are rejected before rmcp can create a transport session. A live,
server-validated MCP session is a trusted task-family boundary, but a fresh
`initialize` creates a new session: exact continuity across fresh sessions would
require a future server-signed persistent logical-task lease and is not claimed
by the current protocol.

A structural envelope does not turn one page into a byte- or CPU-isolated
process. JavaScript or native renderer work can continue after an Agent
operation releases its permit, and shared-Host RSS attribution can be diluted
by sibling tasks. As a physical backstop, when verified managed-Chromium RSS
exceeds the hardware-derived ratio for three consecutive samples and exact
task-local reclaim makes no progress, the Hub replaces the largest attributable
managed Host. CPU has an independent Host-level endpoint: when whole-system CPU
is at least 90% and exact live managed-Chromium process trees account for at
least 50% of machine capacity for three consecutive samples, the Hub replaces
the busiest matched managed Host. A recovery sample, or a successful task-local
close, resets the corresponding streak. These fallbacks never scan or terminate
unrelated system Chrome, but replacing a shared Host necessarily interrupts its
sibling tasks and requires a fresh observe. Exact per-task byte and CPU
enforcement would require a task-dedicated Chromium process tree plus an OS
Job/cgroup; the attributed memory setting and CPU endpoint deliberately do not
claim that isolation.

One managed Host is one Chromium process tree, not one operating-system
process. Chromium normally creates browser, renderer, GPU, network/utility, and
crash-handler child processes. Multiple process rows are therefore expected;
Host count, isolated profile ownership, root PID ancestry, and aggregate tree
RSS are the authoritative diagnostics. Primary and Anonymous Lanes reuse their
corresponding Host; only explicit Isolated identity creates a per-Lane Host.
On Windows, descendant attribution also verifies process start times: an orphan
can retain a stale parent PID after its parent exits, and that numeric PID can
later be reused by Chromium. Rejecting children that predate their current
recorded parent prevents unrelated long-lived process trees from inflating
browser RSS and producing false pressure alerts.
The launch baseline keeps Chromium's native background throttling enabled and
does not weaken site isolation or disable GPU acceleration merely to reduce a
process count. Each Lane is additionally limited to eight top-level tabs, while
the task-wide cap applies across all of its Lanes; explicit excess opens are
rejected and excess page-created popups are closed by the bounded target-cleanup
path. Cross-origin iframes/OOPIFs can still create additional renderer
processes, so this top-level target limit is not a physical process-tree cap.

The shared Anonymous Host has a separate, bounded profile lifecycle so cache,
IndexedDB, CacheStorage, and Service Worker state cannot grow for the lifetime
of the application. By default, the exact Host is fenced and rotated after its
profile reaches 512 MiB or 50,000 entries, after 30 minutes, or before an
admitted navigation would exceed 256. Footprint sampling is bounded and fails
closed; the old Host remains fenced until its exact process tree is stopped and
its ephemeral profile cleanup completes. Anonymous launches also constrain the
ordinary disk and media caches to 64 MiB and 32 MiB. These are per-Host hygiene
limits, not an installation-wide browser-memory cap.

Cross-CDP retained data is bounded independently. Page text is limited to
1 MiB, rendered HTML to 8 MiB, extraction schemas to 64 KiB/32 levels/4,096
nodes, one crawl result to 9 MiB, and the final crawl batch to 16 MiB. A batch
accepts at most 64 URLs with eight workers, while each URL is limited to 16 KiB
and all input URLs together to 256 KiB. Auxiliary CDP sessions are limited to
64 per Lane and 256 per trusted task family; Host-global workers that cannot be
attributed from a trusted browser lineage use a separate 64-session Host bucket.
Rejected sessions are detached and their exact targets closed.

Ephemeral profile deletion itself is resumable rather than all-or-nothing. One
pass retains at most 100,000 entries and 16 MiB of names/paths, keeps the exact
ownership record until the final empty-directory proof, and hands incomplete
work back to authoritative cleanup. Startup recovery may continue the same
claimed profile for up to 64 passes or 30 seconds in one invocation; an active
writer or identity change fails closed and leaves authority for a later retry.

Scheduling follows these constraints:

- an owner's first Lane has weight 4 and expansion Lanes have weight 1;
- when no Lane is globally active and system memory is below the full reserve
  but above the critical floor, at most one basic-availability Lane may start
  against that floor; a second first Lane and every expansion Lane still
  require the full reserve, and critical pressure admits none;
- equal-priority work rotates between owners;
- the task's operation, Lane, tab, and queue quotas are checked independently,
  so saturation in one task does not consume another task's envelope;
- queue age raises effective priority;
- cancellation immediately removes queued work;
- resource balancing does not preempt an operation already in progress; and
- machine-wide pressure reclamation first freezes idle expansion or Crawl
  Lanes, then closes lanes that remain frozen on a later pressured sweep;
  task-budget reclamation targets only the over-budget task and never closes a
  sibling task's Lane on a shared Host.

Automatic idle expiry is 2 minutes and pressure eligibility starts after 30
seconds; Resource saving uses 1 minute and 15 seconds respectively. Both use no
empty-Host warm window. High concurrency retains the older 10-minute normal,
2-minute pressured, and 1-minute empty-Host warm windows. The periodic sweep
remains a recovery backstop for expired owner credentials and stale Lane
lifecycle state, not the normal cleanup path: explicit Lane closure and
Agent-turn completion close the final Host immediately.

For a stable application-managed profile, a proven normal Host shutdown removes
the exact completed ownership marker and that launch's `DevToolsActivePort`
file, while preserving cookies, site storage, and other stable profile data.
Cleanup is fail-closed: if exact process exit or marker ownership cannot be
revalidated, the artifacts are preserved for authoritative recovery and the
operation must not be reported as a clean shutdown.

## Browser management page boundary

`/browser` is the unified browser-management surface, not a page-execution
surface. Its **Lifecycle** tab shows management-safe Lane/Host state, capacity
and queue pressure, identity and owner information, concurrency policy, and
lifecycle state. It can close a Lane, the current user's Lanes for a
conversation, or—when authorized—all Lanes for the installation. For a running
Primary Lane, its owner may make the current managed Host visible in the
foreground or return it to headless background operation without changing the
global default. Its **Settings** tab owns the Browser Use toggle, source, global
Primary display default, login identity, security, and resource policy. The
legacy `/settings/browser-use` route redirects to that tab.

The page does not display browser pixels, mirror a page, navigate, accept page
input, operate tabs, broker a control handoff, or attach a client to Chromium.
It creates no image stream, browser-specific WebSocket, or raw CDP/profile
connection. Under the global `headless` default, ordinary Agent Browser Use
launches Primary with Chromium `--headless=new`; it does not create a hidden or
minimized OS window. Foreground is a one-time display request for the current
Primary Host and never changes the persisted default. Background is its
symmetric current-Host operation. When a live Primary Host must change between
headless and headful—whether for foreground, background, or a default-policy
change—the Hub replaces the whole shared Primary Host at a new browser epoch
and rebinds every live Primary Lane on that Host. The Hub makes a best effort to
restore each Lane's active URL, but process-local target/frame/ref state is
invalid: callers must refresh inventory, perform a fresh observe, and never
reuse old refs. These actions grant no page input or takeover capability.
Closing a Lane through the management page does not close its conversation or
Agent execution; when it was the Host's last Lane, target cleanup is followed
by immediate Host shutdown.

Page execution remains subject to the Agent action and approval model.
Read-only observation is handled as Info work; actions that may change a page,
account, or external state are handled as Exec work and continue through the
application's approval, egress, secret, download, and full-power policies.
Sensitive or irreversible actions remain fail-closed. The management page and
its API do not grant a page-execution capability or bypass these gates.

## Management API and realtime events

The authenticated management API consists of:

- `GET /api/browser/overview`
- `GET /api/browser/lanes`
- `POST /api/browser/lanes/{id}/foreground`
- `POST /api/browser/lanes/{id}/background`
- `POST /api/browser/lanes/{id}/close`
- `POST /api/browser/conversations/{id}/close`
- `POST /api/browser/close-all`
- `GET /api/browser/display-mode`
- `PUT /api/browser/display-mode`
- `GET /api/browser/resource-policy`
- `PUT /api/browser/resource-policy`

`POST /api/browser/lanes/{id}/foreground` is user-scoped and accepts only an
owned Lane whose identity is Primary and whose lifecycle state is `running`.
If the Lane is on a headless Primary Host, the Hub safely replaces that shared
Host with a headful Host using the same application-managed profile, advances
the epoch, and rebinds every live Primary Lane. The Hub makes a best effort to
restore active URLs, but the caller must refresh inventory and perform a fresh
observe. This is a one-time current-Host request: it does not update the global
default, transfer page control, or expose a model-callable Browser operation.
`POST /api/browser/lanes/{id}/background` applies the symmetric headless
transition and also leaves the global default unchanged.

`GET` and `PUT /api/browser/display-mode` are installation-owner controls for
the persisted `headless` or `external` Primary default. A confirmed `PUT`
applies to the live Hub immediately. If a running Primary Host must change
mode, the same new-epoch Host replacement and all-Lane rebind complete before
success is returned; future Primary launches then use the persisted choice.

The installation-owner-only Primary sign-in compatibility flow also exposes
`POST /api/browser/login/open`, `POST /api/browser/login/close`, and
`GET /api/browser/login/status`. These routes allocate and manage a normal Hub
Primary Lane and follow, rather than override, the current global display
default. Under `headless`, the user must explicitly foreground the running Lane
on `/browser` to show it; under `external`, the Primary Host is already visible
by policy. These routes do not create a second browser, embed its page, or grant
control through the Browser page.

Inventory and Lane/conversation lifecycle operations are filtered to the
authenticated user. Installation-wide close and resource-policy operations
and global display-policy operations also require installation-owner authority.
State-changing requests retain the application's normal CSRF protection.

Installation-wide **Close All** is confirmed only when its result contains
`remaining_lane_count = 0`, `remaining_cleanup_count = 0`, and
`remaining_managed_host_count = 0`. A successful HTTP status or a detached Lane
count alone is not completion; a nonzero or missing remaining count is an
unconfirmed/incomplete drain and must be surfaced as such.

Browser inventory and lifecycle changes use the shared authenticated `/ws`
JSON realtime channel. That channel remains a general application event stream;
it carries no Browser image frames or page-input messages.

The former `POST /api/browser/lanes/{lane_id}/return-control`,
`POST /api/browser/lanes/{lane_id}/viewer-token`, and
`GET /api/browser/lanes/{lane_id}/view` routes are superseded historical
interfaces, not current API endpoints.

## Settings migration

The global Primary display default has two trusted values: `headless` and
`external`. New installations persist `headless` together with display-policy
version `2`. Either valid value is preserved when it is explicitly persisted
with version `2`. Every pre-versioned value—including a historical `external` inferred
from the removed `silent=false` setting—and a missing or malformed version-2
mode converges once to `headless`. A preference-store read failure uses
`headless` fail-safe without writing migration state.

The installation owner may subsequently select either value from
`/browser?tab=settings`; a confirmed change is applied live and persisted.
Ordinary Agent action JSON, Lane names, and tool parameters cannot override it.
Anonymous/Crawl, Authenticated Replica, and Isolated Hosts remain headless
under Hub policy unless a separate trusted flow explicitly requires otherwise.

`agent.browserUse.source` chooses whether system Chrome/Edge or a managed source
is preferred. It never authorizes personal-profile reuse and does not change
the Hub's identity, capacity, approval, or lifecycle policies.

## Stable errors

Browser errors include a stable machine code, a safe explanation, retryability,
and a recommended next step, plus Lane, capacity, queue, or recovery metadata
when applicable. Principal codes include:

- `browser_capacity_queued`
- `system_memory_pressure`
- `lane_closed_by_user`
- `owner_lease_expired`
- `stale_browser_epoch`
- `stale_lane_ref`
- `target_crashed`
- `browser_restarted`
- `identity_replica_stale`
- `needs_primary_identity`

Error text must not contain cookies, site-storage values, raw CDP endpoints,
debugging ports, profile paths, or profile contents.
