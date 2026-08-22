# Standalone workbench wiring

This directory supplies two prop-free route components mounted by the
production router at `/workshop/image` and `/workshop/video`:

- `ImageWorkbenchProductRoute`
- `VideoWorkbenchProductRoute`

Each route accepts an explicit `?projectId=<canonical UUIDv7>` scope. With no
scope it renders the source workbench in a fail-closed state and asks the user
to choose a real project; it never borrows or creates a recent project implicitly.
The workbench body owns the same quiet stone token set as the canvas and stays
pixel-stable when the surrounding application theme changes.

The routes never create a canvas config node and never use React-only task ids
as persistence. Every POST uses the exact
`standalone_workbench { projectId, workbenchKind }` owner. The backend owner
index is therefore the only durable task inventory; terminal settlement reloads
that inventory, and a confirmed backend 404 only refreshes the same owner scope.

## Durable task-owner foundation

The canonical task API and migration 043 now define a third owner branch:
`standalone_workbench { projectId, workbenchKind }`, where `workbenchKind` is
exactly image, video, or audio. It is mutually exclusive with canvas-node and
workflow-step ownership, immutable under idempotent replay, and constrained to
the matching media capability family. Every new task also persists and returns
its ordered `{ assetId, kind, role }` input snapshot. A migrated legacy task
whose input kinds cannot be proven returns `inputs: null`; callers must disable
exact retry rather than interpret that value as an empty input list.

The backend exposes the exact owner-scoped list/recovery read path at
`GET /api/creative-studio/tasks?project_id=...&workbench_kind=...`. It uses a
strict newest-first keyset cursor (`submitted_at:creation_task_uuidv7`), audits
every result artifact before returning a page, and rejects unknown query fields,
foreign owners, capability drift, malformed cursors, and limits outside 1-100.
The UI task layer parses that page strictly, bounds every response to the
requested limit/keyset window, and requires a continuation cursor to equal the
last visible task key. The standalone history model merges durable and live
rows without splitting a multi-output task. A live row may replace only the
mutable state of the same immutable task identity, never downgrade a more
advanced durable state, and never attach outputs outside the task's exact
ordered result IDs. Conflicting terminal states fail closed. Retry is allowed
only for failed/canceled tasks whose ordered input snapshot is proven; legacy
`inputs: null` rows remain visible but cannot be retried exactly. The optional
`active_only=true` inventory is paged to exhaustion before the runtime mounts,
so queued/running tasks cannot disappear behind an older visible-history page.

Both product routes now consume that contract. They load the first 30 durable
rows, append older pages only through the returned cursor, merge current live
entries by task identity, and explicitly resume the active inventory. Image
multi-output results remain one task card with ordered media. “载入” hydrates
every input asset by exact ID; “重试” keeps the old record and creates a new
idempotency key from the immutable request snapshot. Queued/running cards use
the canonical cancel route. Cold history is never passed to
`runtime.retry(taskId)`.

## History retirement and asset safety

Migration 044 adds a `deleted_at` tombstone that is separate from task status.
`POST /api/creative-studio/tasks/retire` accepts one exact
`{ project_id, workbench_kind, task_ids }` batch (1-100 IDs), audits every row
and succeeded artifact, and atomically retires only failed/canceled/succeeded
standalone tasks. A queued/running, missing, duplicate, foreign-owner or corrupt
row rejects the whole batch. Repeating the same request preserves the first
tombstone timestamp.

Normal and active history lists exclude tombstones, while direct GET,
idempotent replay, boot inventory and artifact audit retain them. The UI offers
single and multi-select retirement only on terminal cards, requires a fixed-
palette confirmation, then reloads the first page. Runtime entries are dismissed
only after the backend commits. Live cards retain cancel and never expose delete.

Retirement never deletes media. Task inputs and results both restrict asset
deletion, including after retirement; succeeded manifests cannot be shortened
into an invalid empty result. Generated results remain in the asset library.
Projects with queued/running creation tasks cannot be deleted until those tasks
are canceled. Restore, retention purge and optional output cleanup are future
explicit commands, not hidden side effects of this action.

The product does not delete result assets or keep `hiddenIds` to impersonate a
deleted history row.
Earlier `canvas_node` tasks remain auditable canvas tasks and are never guessed
into the standalone owner scope; the new routes contain no compatibility reader
or dual-write path for the retired config-node ledger.

## Exact model retirement

Deleting one catalog model is coordinated with Creative Studio instead of
leaving stale invocation targets behind. Workshop first parses and validates an
exact `{ providerId, model }` cleanup plan. The model repository then applies
every project/workflow replacement with revision CAS, removes the exact model
capabilities and model row, and increments the Provider configuration revision
inside one SQLite writer transaction. A stale project/workflow revision rolls
back the complete operation; there is no partial cleanup followed by a failed
delete and no legacy repository delete bypass.

Only mutable selections are cleared: config nodes, image/video/audio composer
drafts, Workflow generator/planner bindings, and empty Agent sessions. The same
Provider's other models survive. Completed Agent sessions, terminal creation
tasks, generated-asset origin, and terminal Workflow snapshots remain immutable
history. Queued/running tasks, live config owners, pending Agent turns, or
nonterminal Workflow snapshots reject deletion before any write.

A brand-new creation task now proves the Provider and exact model are enabled
and that the model exposes the precise task capability in the same transaction
that inserts the task. Exact idempotent history replay still returns before
those live-parent checks, so deleting a model cannot erase or corrupt audit
history. Image and video workbench selectors clear a vanished exact model after
the canonical catalog refresh; they never substitute another model silently.

## Deliberate blocker

Standalone video still fixes `taskCount` to one. The owner/history foundation
can represent several tasks, but enabling 1-6 fan-out changes cancellation,
selection, retry and provider-cost UX and therefore remains a separate product
gate rather than a side effect of route migration.

The current creation engine executes T2V and single-image I2V, but explicitly
rejects V2V. The standalone video route therefore accepts at most one real
image reference, maps its controlled resolution/aspect selection to concrete
width and height, and does not expose video references, multi-image frames, or
advanced provider parameters until an exact protocol capability contract exists.
