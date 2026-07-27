# Browser Platform Architecture

NomiFun's browser platform is an Agent-oriented managed Chromium service. Its
execution and scheduling unit is a **Browser Lane**, not an Agent session or a
Chromium process. The `/browser` page is a status-only management surface for
lifecycle, concurrency, and capacity; it is not a browser rendering or control
surface.

> **Superseding product decision (2026-07-27):** the former embedded preview,
> JPEG/screencast transport, dedicated viewer WebSocket, viewer token, and user
> takeover/return-control flow have been removed. References to those features
> in historical plans, requirements, or test records are superseded and are not
> current capabilities or work to restore.

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
| `primary` | Interactive browsing, sign-in, and account-affecting work | Uses a stable, application-isolated profile and always runs headful in a visible external managed Chromium window. Primary Lanes share live identity state. |
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
- ACP browser stdio holds only a short-lived, renewable loopback capability
  scoped to an audience and operation set; it does not own Chromium.
- Knowledge URL rendering uses a fixed Anonymous Lane.
- Browser sign-in uses a managed Primary Lane in the external Chromium window.
- HTTP management endpoints invoke only user-scoped inventory and lifecycle
  boundaries or installation-owner resource controls.

The model may select only a length-bounded Lane name. `user_id`, conversation,
runtime instance, attempt, owner lease, allowed operations, and expiry all come
from trusted application context.

When a runtime, attempt, remote connection, or capability ends, its owner lease
must be revoked and that owner's Lanes closed. Application shutdown waits for
the Hub to close all Lanes and Hosts explicitly before process exit completes.

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

When capacity is unavailable, a call returns structured `queued` state rather
than a "ready" handle blocked behind a hidden global lock. Queue metadata
includes position, suggested concurrency, owner/global active and queued
counts, retry delay, and a stable reason code.

## Resources and lifecycle

The Automatic policy derives safe limits from total system memory and logical
CPU count. At runtime, the Hub samples available memory, CPU pressure, and
managed Chromium RSS and computes `normal`, `pressured`, or `critical` state.

Scheduling follows these constraints:

- an owner's first Lane has weight 4 and expansion Lanes have weight 1;
- equal-priority work rotates between owners;
- queue age raises effective priority;
- cancellation immediately removes queued work;
- resource balancing does not preempt an operation already in progress; and
- pressure reclamation prefers idle expansion or Crawl Lanes while protecting
  an owner's only active Lane.

Normal idle expiry is 10 minutes; reclaimable Lanes use a 2-minute idle expiry
under pressure. The periodic sweep also handles expired owner credentials,
Lane lifecycle, and Host warm timers.

## Browser management page boundary

`/browser` shows management-safe Lane/Host state, capacity and queue pressure,
identity and owner information, concurrency policy, and lifecycle state. It can
close a Lane, the current user's Lanes for a conversation, or—when authorized—
all Lanes for the installation and can manage installation-wide resource
policy.

The page does not display browser pixels, mirror a page, navigate, accept page
input, operate tabs, broker a control handoff, or attach a client to Chromium.
It creates no image stream, browser-specific WebSocket, or raw CDP/profile
connection. The actual Primary page remains in the visible external managed
Chromium window. Closing a Lane through the management page does not close its
conversation or Agent execution.

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
- `POST /api/browser/lanes/{id}/close`
- `POST /api/browser/conversations/{id}/close`
- `POST /api/browser/close-all`
- `GET /api/browser/resource-policy`
- `PUT /api/browser/resource-policy`

The installation-owner-only Primary sign-in compatibility flow also exposes
`POST /api/browser/login/open`, `POST /api/browser/login/close`, and
`GET /api/browser/login/status`. These routes allocate and manage a normal Hub
Primary Lane in the visible external Chromium window; they do not create a
second browser, embed its page, or grant control through the Browser page.

Inventory and Lane/conversation lifecycle operations are filtered to the
authenticated user. Installation-wide close and resource-policy operations
also require installation-owner authority. State-changing requests retain the
application's normal CSRF protection.

Browser inventory and lifecycle changes use the shared authenticated `/ws`
JSON realtime channel. That channel remains a general application event stream;
it carries no Browser image frames or page-input messages.

The former `POST /api/browser/lanes/{lane_id}/return-control`,
`POST /api/browser/lanes/{lane_id}/viewer-token`, and
`GET /api/browser/lanes/{lane_id}/view` routes are superseded historical
interfaces, not current API endpoints.

## Settings migration

The product display mode is fixed to `external`. New installations write
`agent.browserUse.displayMode = external`. Historical `embedded`, `headless`,
invalid values, and the old `agent.browserUse.silent` key are compatibility
inputs only and converge to `external`; the old `silent` key is no longer
written. This migration affects Primary only: Primary is always visible and
headful, while Anonymous/Crawl, Authenticated Replica, and Isolated Hosts may
still run headless under Hub policy.

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
