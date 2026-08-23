# Creative Canvas editor wiring

`CreativeCanvasEditor` is the route-ready controller boundary for one canonical
Creative Studio Canvas. It loads by `canvasId`, owns the Canvas reducer and
pointer/keyboard interactions, and saves the complete Canvas document through
revision compare-and-swap (CAS).

## Route composition

The route owns the selected `tool` and supplies product renderers. The editor
does not import a node skin and does not synthesize media or invoke a model.

```tsx
const editorRef = useRef<CreativeCanvasEditorHandle>(null);

<CreativeCanvasEditor
  ref={editorRef}
  canvasId={canvasId}
  tool={tool}
  renderNode={(context) => <ProductNodeView {...context} />}
  renderEdge={(context) => <ProductConnectionView {...context} />}
  leftPanel={(context) => <CanvasNavigator state={context.state} />}
  rightPanel={(context) => <Inspector state={context.state} />}
/>
```

Product chrome sends every Canvas mutation through the same reducer and CAS
boundary with `editorRef.current?.dispatch(command)`. Background changes use
`editorRef.current?.setBackground('dots' | 'lines' | 'blank')`; the editor
updates the canonical Canvas document and queues the complete document through
the same save controller. Product routes must not synthesize keyboard events or
maintain a second Canvas store.

Panel chrome uses `setPanels(nextPanels)`, and inspector edits dispatch the
complete discriminated node returned by `canvasCommands.updateNode(...)`.
Both paths merge with the latest reducer-owned nodes and viewport before they
queue the same full-document CAS save.

`renderNode` receives the canonical Canvas node, its selected state, `onActivate`, and
pointer props for the product's chosen drag handle. `renderEdge` receives the
canonical connection plus its resolved source and target Canvas nodes. Top, left,
right, bottom, screen-overlay, and minimap slots can be static nodes or functions
of the current editor context.

## Leaving and save conflicts

Before route navigation, await `editorRef.current?.flush()`. Continue for
`status: 'noop'` or `status: 'saved'`; keep the editor mounted for `conflict` or
`error` so the user can resolve it. While a hydrated revision still has pending
changes, the browser `beforeunload` listener starts the same flush and requests
the native leave confirmation instead of silently allowing Ctrl+R/close during
the debounce window. Browsers still cannot await asynchronous persistence
during unload, so accepting that warning is an explicit choice to leave before
the save is known durable. Tauri's normal close gesture only hides the main
window, leaving the renderer and save controller alive.

A revision conflict is terminal for the current save controller. It never
retries with a newer revision and never overwrites remote state. The built-in
conflict action calls `reloadRemote()`, which explicitly discards local canvas
changes and hydrates the authoritative server revision. A product-specific
merge flow can be added above this boundary later; it must still end in an
explicit user choice.

Only HTTP 409 responses carrying the stable backend code
`REVISION_CONFLICT` enter that terminal state. Other business conflicts retain
their own error path and must never be presented as a reason to discard local
work. Returning the local document to its pre-edit signature also does not
clear a conflict; only `reset(remoteRevision, remoteDocument)` may unlock the
controller.

## Product integrations left above this boundary

Node creation palettes, handles for creating connections, media preview and
asset resolution, model selection/execution, task progress, Agent transport, and inspector
forms remain route/product responsibilities. They should dispatch canonical
core commands or update the canonical Canvas document; no legacy workshop
schema adapter belongs in the editor.

## Agent session references

Conversation contents remain owned by NomiFun, while the Canvas document keeps
only validated session/model/message references and one durable pending-turn
fence. Product code must call and await
`persistAgentSessions(sessions, activeSessionId)` before submitting a turn and
again after authoritative history reconciliation. The method merges with the
latest canvas reducer state, validates the complete canonical v1 document,
flushes immediately, and rejects on CAS or transport failure.

`onAgentSessionsChange` publishes hydration and local durable mutations;
`getAgentSessions()` and `getActiveAgentSessionId()` expose the same snapshot to
imperative transport adapters. No Agent adapter may write Canvas state outside
this Editor/CAS boundary.

Every owner-bearing task or Agent reference is scoped by the canonical
`CanvasNode { canvasId, nodeId }` identity. HTTP owner fields use
`canvas_id`/`node_id`; a node ID without its Canvas ID is not an acceptable
owner.

## Pending task recovery feed

The canonical `pendingTaskIds` array is owned by this same Editor/CAS boundary.
A workbench controller must await `addPendingTask(taskId)` before POST, then
await `removePendingTask(taskId)` only after a terminal task or a confirmed 404
orphan. Both methods flush immediately and reject on CAS/transport failure, so
the runtime cannot continue past a merely queued local mutation.
`onPendingTaskIdsChange` fires once after remote hydration and after
each local feed mutation; `getPendingTaskIds()` provides the same snapshot to
imperative adapters.

For pending `image-node-compose`, the command guard protects both the config
owner and its persisted `sourceNodeId`. User delete/update/undo commands cannot
remove the source or replace its `assetId` while the task is live. The explicit
`node/reconcile-runtime` path remains allowed so the authoritative terminal
result can fill an empty source before the pending ID is removed.

For a future standalone workbench route, build `initialResumeRequests` only
after the hydration callback, using real config-node identity/model/capability
data plus the returned IDs. Wire the runtime's `onPendingTask` and
`onSettledTask` to the two Editor methods above; its orphan callback returns
`true` only after `removePendingTask` resolves:

```tsx
onPendingTask: (reference) => editor.addPendingTask(reference.taskId),
onSettledTask: (task) => editor.removePendingTask(task.taskId),
onRecoveryFailure: async (reference) => {
  await editor.removePendingTask(reference.taskId);
  return true;
},
```

The Editor layer does not mount a workbench runtime. The canvas product uses
this seam for its one Canvas-scoped image runtime; other products must wire
their own runtime explicitly rather than assuming recovery is automatic.

## Compatibility only

The coordinated `nomifun.creative-studio/v1` reader may still encounter an
internal `projectId` in the historical document shape. Legacy project-document
and repository adapters may translate that shape at the migration boundary,
but the Editor contract above remains Canvas/canvasId-based and must not expose
those names as product terminology.
