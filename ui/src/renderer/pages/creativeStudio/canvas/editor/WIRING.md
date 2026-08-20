# Creative canvas editor wiring

`CreativeCanvasEditor` is the route-ready controller boundary for a canonical
creative-studio project. It loads through `useCreativeProject`, owns the canvas
reducer and pointer/keyboard interactions, and saves the full canonical document
through revision compare-and-swap (CAS).

## Route composition

The route owns the selected `tool` and supplies product renderers. The editor
does not import a node skin and does not synthesize media or invoke a model.

```tsx
const editorRef = useRef<CreativeCanvasEditorHandle>(null);

<CreativeCanvasEditor
  ref={editorRef}
  projectId={projectId}
  tool={tool}
  renderNode={(context) => <ProductNodeView {...context} />}
  renderEdge={(context) => <ProductConnectionView {...context} />}
  leftPanel={(context) => <ProjectNavigator state={context.state} />}
  rightPanel={(context) => <Inspector state={context.state} />}
/>
```

Product chrome sends every canvas mutation through the same reducer and CAS
boundary with `editorRef.current?.dispatch(command)`. Background changes use
`editorRef.current?.setBackground('dots' | 'lines' | 'blank')`; the editor
updates the canonical base document and queues the complete document through
the same save controller. Product routes must not synthesize keyboard events or
maintain a second canvas store.

Panel chrome uses `setPanels(nextPanels)`, and inspector edits dispatch the
complete discriminated node returned by `canvasCommands.updateNode(...)`.
Both paths merge with the latest reducer-owned nodes and viewport before they
queue the same full-document CAS save.

`renderNode` receives the canonical node, its selected state, `onActivate`, and
pointer props for the product's chosen drag handle. `renderEdge` receives the
canonical connection plus its resolved source and target nodes. Top, left,
right, bottom, screen-overlay, and minimap slots can be static nodes or functions
of the current editor context.

## Leaving and save conflicts

Before route navigation, await `editorRef.current?.flush()`. Continue for
`status: 'noop'` or `status: 'saved'`; keep the editor mounted for `conflict` or
`error` so the user can resolve it. The browser `beforeunload` listener is only
a best-effort flush because browsers cannot await asynchronous persistence while
closing.

A revision conflict is terminal for the current save controller. It never
retries with a newer revision and never overwrites remote state. The built-in
conflict action calls `reloadRemote()`, which explicitly discards local canvas
changes and hydrates the authoritative server revision. A product-specific
merge flow can be added above this boundary later; it must still end in an
explicit user choice.

## Product integrations left above this boundary

Node creation palettes, handles for creating connections, media preview and
asset resolution, model selection/execution, task progress, Agent transport, and inspector
forms remain route/product responsibilities. They should dispatch canonical
core commands or update the canonical project document; no legacy workshop
schema adapter belongs in the editor.

## Agent session references

Conversation contents remain owned by NomiFun, while the project document keeps
only validated session/model/message references and one durable pending-turn
fence. Product code must call and await
`persistAgentSessions(sessions, activeSessionId)` before submitting a turn and
again after authoritative history reconciliation. The method merges with the
latest canvas reducer state, validates the complete canonical v1 document,
flushes immediately, and rejects on CAS or transport failure.

`onAgentSessionsChange` publishes hydration and local durable mutations;
`getAgentSessions()` and `getActiveAgentSessionId()` expose the same snapshot to
imperative transport adapters. No Agent adapter may write project state outside
this Editor/CAS boundary.

## Pending task recovery feed

The canonical `pendingTaskIds` array is owned by this same Editor/CAS boundary.
A workbench controller must await `addPendingTask(taskId)` before POST, then
await `removePendingTask(taskId)` only after a terminal task or a confirmed 404
orphan. Both methods flush immediately and reject on CAS/transport failure, so
the runtime cannot continue past a merely queued local mutation.
`onPendingTaskIdsChange` fires once after remote hydration and after
each local feed mutation; `getPendingTaskIds()` provides the same snapshot to
imperative adapters.

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

The canvas product does not currently mount a workbench runtime, so this seam
does not claim automatic recovery is active here.
