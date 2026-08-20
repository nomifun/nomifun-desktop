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
asset resolution, model selection/execution, task progress, chat, and inspector
forms remain route/product responsibilities. They should dispatch canonical
core commands or update the canonical project document; no legacy workshop
schema adapter belongs in the editor.
