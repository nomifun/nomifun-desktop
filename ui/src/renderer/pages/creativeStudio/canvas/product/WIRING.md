# Creative Canvas product route wiring

This directory exports a default, no-props route component. It reads the
canonical `projectId` through `useParams` and keeps `CreativeCanvasEditor` as
the only reducer and CAS persistence owner.

Add the lazy import next to the other Creative Studio route imports:

```tsx
const CreativeCanvasProductRoute = React.lazy(
  () => import('@renderer/pages/creativeStudio/canvas/product')
);
```

Then add the nested route inside the existing
`CREATIVE_STUDIO_ROOT_PATH` / `CreativeStudioFocusShell` route:

```tsx
<Route
  path='canvas/:projectId'
  element={withRouteFallback(CreativeCanvasProductRoute)}
/>
```

The product's own “返回项目” action awaits the editor CAS `flush()` and only
navigates to `CREATIVE_STUDIO_PROJECTS_PATH` after `noop` or `saved`. A
`conflict` or `error` stays on the canvas and exposes explicit reload/retry.

The existing declarative `HashRouter` does not provide this product component
with a safe, asynchronous global route blocker. Links owned by the surrounding
`CreativeStudioFocusShell` can bypass the product callback. If every shell or
browser navigation must await CAS, the shell/router integration must await the
exported `requestCreativeCanvasProductBeforeLeave()` and navigate only when it
returns `true`. The product route registers and unregisters its active Editor
flush gate automatically; do not create a second persistence controller.

Import the lightweight coordination function from
`@renderer/pages/creativeStudio/canvas/product/beforeLeave` so the focused
shell does not eagerly load the product route chunk.

Panel open/view changes call the Editor's canonical `setPanels` port; saved
width/height values also drive the product layout. Properties dispatch the
type-safe core `node/update` command and therefore participate in normal undo,
CAS save, conflict, and reload behavior. Background changes use the same CAS
port. The right-side Agent uses the owner-only Creative Studio session resolver,
the real NomiFun Conversation REST/WebSocket transport, and Editor-owned CAS for
session references and response-loss fences. Route exit first stops and settles
an active exclusive Agent turn, then flushes the Editor. The left workflow panel
uses the canonical workflow repository and durable run controller, opens the
same typed runner and real asset picker as the standalone center, and resolves
successful result IDs through the authenticated asset-detail endpoint before
inserting canonical nodes. The global timeline remains an explicit unavailable
state until its production projection is connected.

The source geometry is canonical for views that currently have no resize handle:
the left library is 280px and opening Agent normalizes the right panel to 390px.
The Agent supplies its own single header; the generic right-panel tab header is
shown only for properties so the product never renders stacked title bars.

The Editor also exposes the canonical pending-task recovery feed described in
`../editor/WIRING.md`. This product does not currently instantiate a workbench
runtime, so it intentionally does not fabricate `initialResumeRequests`.
