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

The current image/video routes intentionally keep their config-node persistence
until the next history slice adds owner-scoped list/recovery. Switching POST
ownership before that read path exists would make an in-flight task impossible
to recover after reload. No product code should create another owner adapter in
the meantime.

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
