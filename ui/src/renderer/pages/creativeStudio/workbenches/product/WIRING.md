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

Task-history deletion remains unavailable until the next atomic backend slice.
The product therefore hides selection/deletion controls; it does not delete
result assets or keep `hiddenIds` to impersonate a deleted history row.
Earlier `canvas_node` tasks remain auditable canvas tasks and are never guessed
into the standalone owner scope; the new routes contain no compatibility reader
or dual-write path for the retired config-node ledger.

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
