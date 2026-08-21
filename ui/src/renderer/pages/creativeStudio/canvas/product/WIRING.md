# Creative Canvas product route wiring

This directory exports a default, no-props route component. It reads the
canonical `projectId` through `useParams` and keeps `CreativeCanvasEditor` as
the only reducer and CAS persistence owner.

The production router mounts this default export at
`/workshop/canvas/:projectId` inside `CreativeStudioFocusShell`.
It keeps the route split with
`import('@renderer/pages/creativeStudio/canvas/product')` and the nested
`path="canvas/:projectId"` contract.

The product's own “返回项目” action awaits the editor CAS `flush()` and only
navigates to `CREATIVE_STUDIO_PROJECTS_PATH` after `noop` or `saved`. A
`conflict` or `error` stays on the canvas and exposes explicit reload/retry.

The surrounding `CreativeStudioFocusShell` awaits the exported
`requestCreativeCanvasProductBeforeLeave()` before product navigation. The
product route registers and unregisters its active Editor flush gate
automatically; do not create a second persistence controller.

The shell imports only the lightweight coordination function from
`@renderer/pages/creativeStudio/canvas/product/beforeLeave`, so it does not
eagerly load the product route chunk.

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
inserting canonical nodes. The bottom timeline now projects the project's one
canonical Director node without inventing global tracks or keyframes. It shows
the saved scene pointer, camera pointer and timeline/duration values, and opens
the real Director product only after the canvas CAS leave gate succeeds. New UI
creation paths enforce one Director node per project; malformed documents with
multiple Director nodes remain visible as a fail-closed conflict. The Director
close action returns to this same canvas project.

The source geometry is canonical for views that currently have no resize handle:
the left library is 280px and opening Agent normalizes the right panel to 390px.
The Agent supplies its own single header; the generic right-panel tab header is
shown only for properties so the product never renders stacked title bars.

The Editor also exposes the canonical pending-task recovery feed described in
`../editor/WIRING.md`. Image-node local editing now instantiates one scoped
workbench runtime only after the project and Editor graph are both hydrated.
It uploads the blue-marked reference as a hidden real asset, persists the
locked config owner plus `pendingTaskIds` before POST, and submits the exact
`image_edit` / `i2i` model identity through the shared NomiFun task client.
Queued/running state is reconciled without creating undo history; terminal
results are fetched as real image assets and written as config-to-image nodes
before pending removal performs the final CAS flush. Mount recovery is derived
only from matching `image-mask-edit` config nodes. A transport-ambiguous create
keeps the draft and idempotency key locked for safe retry; abandonment first
probes the backend and is allowed only after an authoritative 404.
