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

## Deliberate blocker

The v1 config-node schema owns one `taskId`, while video runtime can otherwise
fan out to six task ids. This product therefore fixes standalone video
`taskCount` to one. Enabling parallel standalone videos requires a canonical
`standalone_workbench` owner union (or one explicit config-node owner per task)
across the schema, task client, backend ownership checks, and archive contract.
The UI reports this limit and never writes an invalid pending-task document.

The current creation engine executes T2V and single-image I2V, but explicitly
rejects V2V. The standalone video route therefore accepts at most one real
image reference, maps its controlled resolution/aspect selection to concrete
width and height, and does not expose video references, multi-image frames, or
advanced provider parameters until an exact protocol capability contract exists.
