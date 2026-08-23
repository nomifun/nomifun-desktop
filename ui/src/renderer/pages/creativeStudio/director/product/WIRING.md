# Creative Director product route wiring

The default no-props route is mounted at `/workshop/director/:canvasId`. It
reads the canonical `canvasId` with `useParams`, loads the root Canvas through
the Canvas repository, and saves its Director scene through revision CAS.

The root Canvas document remains authoritative. Its single `director` node
stores a `sceneId` pointer to an immutable, non-library NomiFun text asset.
Every pointer advance uses the root Canvas's compare-and-swap revision;
conflicts never force-write.

The product registers an asynchronous leave gate. Focus-shell navigation
already awaits `requestCreativeDirectorProductBeforeLeave()` and continues only
when it returns `true`, just as it does for the canvas product.
The Director close action awaits the same gate and returns to
`/workshop/canvas/:canvasId`, preserving the current Canvas context instead of
dropping the user at the Canvas library `/workshop/canvases`.

The current Creative Studio asset backend accepts image, video and audio uploads but
not GLB/glTF. The route therefore keeps model/character import explicitly
unavailable instead of substituting unlicensed or fake geometry. Single-frame
Three.js screenshots are uploaded through the real NomiFun asset client. An
individual screenshot or all screenshots for the selected camera can be sent
to the root Canvas as canonical image nodes. The route flushes the Director
sidecar first, resolves every asset through the authenticated asset client,
then saves the root document with exact revision CAS. Existing asset nodes are
reused, and an uncertain response is reconciled against the authoritative
Canvas before retry, so the action cannot create duplicates. Multi-angle
capture and video export remain visibly unavailable.

Canvas ZIP export/import closes over the Director `sceneId` sidecar and every
stable asset referenced by the Director scene: panorama, character, object, and
capture records, including captures that were never sent back to the Canvas.
The outer Director pointer, known image/video/audio composer and mask
`config.operation` node/asset references, graph identities, manifest checksums,
and content paths are remapped from the same complete identity maps.

The archive parser rejects missing dependencies, unknown reference-bearing Canvas
operations, invalid Director envelopes, stale Canvas ownership, undeclared
entries, checksum drift, and ZIP budget violations. Export, Canvas/asset
deletion protection, and startup managed-data audit share the same Director
asset-closure resolver, so hidden scene dependencies cannot be deleted or
omitted independently of their Canvas.

## Compatibility only

The Director sidecar format `nomifun.director.project` v1 is a historical
compatibility format, not the public Director or Canvas product identity. Its
internal root reference may still be named `projectId`; import/export may
rewrite that field and nested asset IDs while preserving its camera, entity, and
timeline identities. Legacy project repository adapters may also be used at
this boundary. All current route and owner terminology remains
`/workshop/director/:canvasId`, Canvas, `canvasId`, and `canvas_id`.

Director owner-bearing records use `CanvasNode { canvasId, nodeId }` and the
wire form `{ kind: 'canvas_node', canvas_id, node_id }`; the v1 sidecar's
historical field names must not leak into that owner contract.
