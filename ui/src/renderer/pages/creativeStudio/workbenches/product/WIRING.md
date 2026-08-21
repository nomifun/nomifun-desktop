# Standalone workbench wiring

This directory supplies two prop-free route components mounted by the
production router at `/workshop/image` and `/workshop/video`:

- `ImageWorkbenchProductRoute`
- `VideoWorkbenchProductRoute`

Each route accepts an explicit `?projectId=<canonical UUIDv7>` scope. With no
scope it renders the source workbench in a fail-closed state and asks the user
to choose a real project; it never borrows or creates a recent project implicitly.

Before POSTing a task the route creates or updates a visible canonical config
node through project CAS. `onPendingTask` durably records the task id before
POST, `onSettledTask` removes it at every terminal state, mount recovery comes
from `document.pendingTaskIds`, and an individual backend 404 removes only that
orphan after a successful CAS save.

## Durable task-owner foundation

The canonical task API and migration 043 now define a third owner branch:
`standalone_workbench { projectId, workbenchKind }`, where `workbenchKind` is
exactly image, video, or audio. It is mutually exclusive with canvas-node and
workflow-step ownership, immutable under idempotent replay, and constrained to
the matching media capability family. Every new task also persists and returns
its ordered `{ assetId, kind, role }` input snapshot. A migrated legacy task
whose input kinds cannot be proven returns `inputs: null`; callers must disable
exact retry rather than interpret that value as an empty input list.

The backend now exposes the exact owner-scoped read path at
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
`inputs: null` rows remain visible but cannot be retried exactly.

The current image/video routes still intentionally keep their config-node
persistence until the coordinated route-integration slice consumes this read
path and switches POST ownership at the same time. Switching POST ownership
without mounting list/recovery in the product route would still make an
in-flight task impossible to recover after reload. No product code should
create another owner adapter in the meantime.

## Deliberate blocker

The v1 route still persists one config-node `taskId`, while video runtime can
otherwise fan out to six task ids. This product therefore fixes standalone
video `taskCount` to one. Enabling parallel standalone videos now requires the
routes to switch to the durable owner above and consume its owner-scoped
history/recovery API; the UI must not fan out before that coordinated slice.

The current creation engine executes T2V and single-image I2V, but explicitly
rejects V2V. The standalone video route therefore accepts at most one real
image reference, maps its controlled resolution/aspect selection to concrete
width and height, and does not expose video references, multi-image frames, or
advanced provider parameters until an exact protocol capability contract exists.
