# Standalone Workbench Wiring

This directory supplies two prop-free route components mounted by the
production router at `/workshop/image` and `/workshop/video`:

- `ImageWorkbenchProductRoute`
- `VideoWorkbenchProductRoute`

Standalone routes have no Canvas query, selector, or parent-load gate. They
are fully usable with zero Canvases and never create or select a hidden Canvas.
The workbench body owns the same quiet stone token set as the Canvas and stays
pixel-stable when the surrounding application theme changes.

Every new task uses the exact installation-owned
`standalone_workbench { workbenchKind }` owner. The backend owner index is the
only durable task inventory; terminal settlement reloads that inventory, and
a confirmed backend 404 refreshes the same workbench-kind scope.

## Session Draft Continuity

Image and video keep one versioned `sessionStorage` draft per
`workbenchKind`. The key contains neither `projectId` nor `canvasId`, so route
continuity cannot introduce a hidden Canvas relationship. A draft saves only
the prompt, exact `{ providerId, model }` identity, controlled generation
parameters, ordered reference asset IDs, and the side/bottom layout.

Busy state, errors, open modals, current selections, task state, and full asset
objects are deliberately excluded. Reads validate the complete v1 shape,
canonical Provider/asset IDs, bounded lengths, allowed parameter values, and
the workbench discriminator. Corrupt, oversized, unknown-version, or
cross-workbench values are discarded without preventing the route from
loading; unavailable browser storage is treated as an empty draft.

On route entry, reference IDs are hydrated individually through the canonical
asset `get` API. Missing, unreadable, mismatched, duplicate, excess, or
non-image references are removed fail-closed rather than represented by stale
browser objects. Generation remains disabled until this initial hydration
finishes. Once the model catalog is ready, the saved model survives only if
that exact Provider/model pair still supports the required task; a same-named
model from another Provider is never substituted.

## Durable Task Owner

The canonical task API and migration 047 define standalone ownership by
`workbench_kind` only. It is mutually exclusive with CanvasNode and
WorkflowStep ownership, immutable under idempotent replay, and constrained to
the matching media capability family. Historical standalone rows may retain a
legacy `project_id` value as inert provenance, but it is ignored by owner
equality, history paging, retirement, asset origin matching, and Canvas
deletion.

Every new task persists and returns its ordered `{ assetId, kind, role }` input
snapshot. A migrated legacy task whose input kinds cannot be proven returns
`inputs: null`; callers must disable exact retry rather than interpret that
value as an empty input list.

The backend exposes the standalone history path at
`GET /api/creative-studio/tasks?workbench_kind=...`. It uses a strict
newest-first keyset cursor (`submitted_at:creation_task_uuidv7`), audits every
result artifact before returning a page, and rejects unknown query fields,
foreign workbench kinds, capability drift, malformed cursors, and limits
outside 1-100.

The UI task layer parses that page strictly, bounds every response to the
requested limit/keyset window, and requires a continuation cursor to equal the
last visible task key. The standalone history model merges durable rows from
all historical provenance buckets with live rows by task identity. A live row
may replace only mutable state of the same immutable task identity, never
downgrade a more advanced durable state, and never attach outputs outside the
task's exact ordered result IDs. Conflicting terminal states fail closed.

Retry is allowed only for failed/canceled tasks whose ordered input snapshot is
proven; legacy `inputs: null` rows remain visible but cannot be retried
exactly. The optional `active_only=true` inventory is paged to exhaustion
before the runtime mounts, so queued/running tasks cannot disappear behind an
older visible-history page.

## History Retirement And Asset Safety

`POST /api/creative-studio/tasks/retire` accepts one exact
`{ workbench_kind, task_ids }` batch (1-100 IDs), audits every row and
succeeded artifact, and atomically retires only failed/canceled/succeeded
standalone tasks. A queued/running, missing, duplicate, foreign-workbench, or
corrupt row rejects the whole batch. Repeating the same request preserves the
first tombstone timestamp.

Normal and active history lists exclude tombstones, while direct GET,
idempotent replay, boot inventory, and artifact audit retain them. The UI
offers single and multi-select retirement only on terminal cards, requires a
fixed-palette confirmation, then reloads the first page. Runtime entries are
dismissed only after the backend commits. Live cards retain cancel and never
expose delete.

Retirement never deletes media. Task inputs and results both restrict asset
deletion, including after retirement; succeeded manifests cannot be shortened
into an invalid empty result. Generated results remain in the global asset
library. Canvas deletion is blocked only by live CanvasNode tasks for that
Canvas; standalone live tasks do not participate.

The product does not delete result assets or keep `hiddenIds` to impersonate a
deleted history row. Earlier CanvasNode tasks remain auditable Canvas tasks and
are never guessed into the standalone owner scope.

## Exact Model Retirement

Deleting one catalog model is coordinated with Creative Studio instead of
leaving stale invocation targets behind. Workshop first parses and validates an
exact `{ providerId, model }` cleanup plan. The model repository then applies
every Canvas/Workflow replacement with revision CAS, removes the exact model
capabilities and model row, and increments the Provider configuration revision
inside one SQLite writer transaction.

Only mutable selections are cleared: Canvas config nodes, image/video/audio
composer drafts, Workflow generator/planner bindings, and empty Agent
sessions. Completed Agent sessions, terminal creation tasks, generated-asset
origin, and terminal Workflow snapshots remain immutable history.

A brand-new creation task proves the Provider and exact model are enabled and
that the model exposes the precise task capability in the same transaction
that inserts the task. Exact idempotent history replay still returns before
those live-parent checks, so deleting a model cannot erase or corrupt audit
history. Image and video selectors clear a vanished exact model after the
canonical catalog refresh; they never substitute another model silently.

## Deliberate Blocker

Standalone video still fixes `taskCount` to one. Enabling 1-6 fan-out changes
cancellation, selection, retry, and provider-cost UX and remains a separate
product gate.

The current creation engine executes T2V and single-image I2V, but explicitly
rejects V2V. The standalone video route therefore accepts at most one real
image reference, maps its controlled resolution/aspect selection to concrete
width and height, and does not expose video references, multi-image frames, or
advanced provider parameters until an exact protocol capability contract exists.
