# Computer Use Architecture

[简体中文](computer-use.zh.md)

Computer Use runs through NomiFun's private worker and the vendored Cua Driver. The host does not link native desktop drivers or create another execution ledger. AgentSession, Turn, Event, Payload and Effect remain the sole Agent fact chain. This page describes the current interfaces; platform and release evidence is recorded in the [active progress section](../handoffs/computer-use-rebuild.zh.md#12-单份执行进展).

## Frozen registration

The package is `nomifun.computer`, mount `domain-computer`, capability `computer`, and role `system.computer_use`. The resource kind is `computer`, the local resource ID is `local-desktop`, and resource operations are `observe`, `input` and `launch`.

The [frozen inventory](../../crates/backend/nomifun-agent-contracts/contracts/computer-tools.v1.json) comes from actual platform worker lists. The [compact overrides](../../crates/backend/nomifun-agent-contracts/contracts/computer-tools.overrides.v1.json) own localized presentation, loading groups, permission metadata and parameter removal. `scripts/computer/generate-contracts.mjs` derives all 22 exact Actions, typed Wave2 operations, presentation records and both official template grants. Hosts, UI and documentation must not maintain another action catalogue.

Canonical authorization accepts the captured native schema forms. Model tools use the exact frozen schema for the actual host platform. Windows and Linux captures exist; macOS capture remains a platform task. Missing capture evidence must not be reported as native acceptance.

## Invocation and process ownership

Calls pass through Kernel → Wave2 → `router/computer_host.rs` → `nomifun-computer` → `nomifun-computer-worker`. The host checks the admitted role, provider, Snapshot, resource owner, Action and resource operation. Each batch step remains constrained by that invocation's exact frozen Action allowlist.

The host crate separates `protocol`, `worker`, `result`, `surface` and `permissions`. The worker has an independent Cargo workspace at `apps/computer-worker`. The host spawns it directly and uses inherited stdin/stdout with upstream private JSON-line protocol v1. There is no daemon, network control endpoint or new event channel.

The worker starts lazily, binds driver sessions to its generation and dispatches one request at a time with fair scheduling between sessions. Stop withdraws queued work. In-flight work retains its actual owner until a result or its action deadline; forced termination produces unknown. A later call may create a new generation and rebind, but never replays the interrupted action. An idle worker without sessions retires automatically. Application shutdown settles the worker before database closure; dropping a cache entry or Future does not prove native cleanup.

For each binding, the private worker appends a fresh UUID to the visible Agent name, giving the native session its own identity. Agents with the same name and later Turn transports therefore cannot share an ended Core session. The worker explicitly calls `start_session`. Closing first awaits successful `end_session` while the exact transport remains authorized, then calls SDK `close` to revoke authorization and removes the binding. Failed initialization or cleanup retains that exact owner for retirement; authorization revocation alone does not prove native teardown.

Applications belong to the user and must survive session closure and worker retirement. The registered Windows launch patch uses a short-lived private helper to put actual user applications under the interactive shell's lifecycle while retaining worker process termination limits. This mechanism and normal Windows verification do not establish native lifecycle acceptance on other platforms; their evidence remains separate.

## Results and model surface

Observation does not create an Effect. Mutations use the existing canonical Effect chain with no Computer `resource_key`. Confirmed, partial, unverifiable and suspected-noop outcomes preserve driver evidence. Pre-dispatch refusal and queued withdrawal express zero effect; crashes, lost transport and forced timeouts express unknown. Delivery alone does not establish task completion: observe and verify the user's postcondition.

Without ToolCalls, no Computer tools are advertised. Without ImageInput, the host projects the accessible no-image surface and rejects pixel addressing. Image-capable routes receive the full surface. Names, titles and prompt text derive from frozen inputs; deferred tools use existing tool discovery. The host strips or injects trusted session, image-size and internal fields. Images follow existing typed image and history limits, including attachment messages after tool results for OpenAI Chat.

A model interface can explicitly set `input_constraints.image_input: false` through its [Chat input constraints](model-providers.md#model-input-constraints-and-agent-routes), giving that candidate a genuine NoImage route. Missing or empty constraints retain protocol defaults; `true` cannot expand unsupported protocol, candidate or adapter input. The surface follows each actual route's effective features. Neither a `vision_input` trait nor a model name grants or disables image input. Setting `include_screenshot: false` on a Full route suppresses that observation's screenshot while the route remains Full.

The Broker projects tools and instructions again for each actual route and adapter, including retries and failover. Projection preserves request identity and historical messages; image history cannot be rewritten to admit a route without image support. A process-local attempt hook records the projected surface before encoding and binds validated tool-call evidence before delivery to Runtime. This evidence checks the schema actually sent for that attempt; it grants no execution authority and creates no additional persistent ledger.

## Permissions and platforms

Frozen `system_permissions` metadata determines live per-call checks. Missing access uses the existing `SYSTEM_PERMISSION_REQUIRED` carrier and does not report execution. Driver status is `GET /api/computer/status`. System access remains under `/api/system/permissions` and its request/open-settings endpoints; old Computer permission aliases are removed.

Status reads the current generation's cache without starting a worker or retrying a probe. `backend` identifies the initialized native dispatch family, not permission, UIA, capture or universal action readiness. Linux `atspi` comes from that worker's read-only accessibility-bus probe at initialization. Missing or failed diagnostics stay unknown and retire with the generation; each action still relies on its current checks and native result.

macOS targets 14+ with direct App-to-worker spawning and the same distribution signing identity. Actual TCC attribution and installed behavior still require platform verification. Windows uses the current login session at ordinary integrity; elevated targets retain structured driver refusal rather than an administrator desktop bridge. Linux currently uses X11/XWayland and AT-SPI. Native Wayland capture remains disabled; portal support, desktop backends and the GNOME helper require their own evidence.

The vendor source is pinned to commit `4d959bbfa57f0c89f310330e849500b222084194`. Changes are applied through the [registered patches](../../vendor/CUA-DRIVER-PATCHES.md) and sync script. The Linux input guardian is internal native resource ownership in the worker, not host desktop code or a second execution ledger. Its final failure matrix still requires separate acceptance.

## Upgrade and distribution

Sessions referencing retired Computer contracts are not converted or repaired. Existing contract rejection asks users to create a new session while keeping history. Personal Agents must select the new Actions again; official General and Companion templates grant the frozen surface to new sessions. Canonical storage and its data generation do not change.

Platform builds compile the independent worker first and stage Cargo's actual executable as a Tauri externalBin. Development sets `NOMIFUN_COMPUTER_WORKER`; the macOS development App uses its signed in-bundle copy. Sign the worker before the enclosing App. Certificates, notarization, installation packages, live models and native platform behavior require distinct evidence. UI/API contract version is 44.
