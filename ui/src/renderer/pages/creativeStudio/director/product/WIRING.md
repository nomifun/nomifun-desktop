# Creative Director product route wiring

The default no-props route is mounted at `/workshop/director/:projectId`. It
reads the canonical `projectId` with `useParams`, loads the root Creative
Studio project through `creativeProjectRepository`, and saves its Director
scene through revision CAS.

The root canvas document remains authoritative. Its single `director` node
stores a `sceneId` pointer to an immutable, non-library NomiFun text asset whose
content is the validated `nomifun.director.project` v1 document. Every pointer
advance uses the root project's compare-and-swap revision; conflicts never
force-write.

The product registers an asynchronous leave gate. Focus-shell navigation
already awaits `requestCreativeDirectorProductBeforeLeave()` and continues only
when it returns `true`, just as it does for the canvas product.
The Director close action awaits the same gate and returns to
`creativeStudioCanvasProjectPath(projectId)`, preserving the current project's
canvas context instead of dropping the user at the project index.

The current Creative Studio asset backend accepts image, video and audio uploads but
not GLB/glTF. The route therefore keeps model/character import explicitly
unavailable instead of substituting unlicensed or fake geometry. Single-frame
Three.js screenshots are uploaded through the real NomiFun asset client. An
individual screenshot or all screenshots for the selected camera can be sent
to the root canvas as canonical image nodes. The route flushes the Director
sidecar first, resolves every asset through the authenticated asset client,
then saves the root document with exact revision CAS. Existing asset nodes are
reused, and an uncertain response is reconciled against the authoritative
project before retry, so the action cannot create duplicates. Multi-angle
capture and video export remain visibly unavailable.

Project ZIP export/import closes over the Director `sceneId` text sidecar and
every stable asset referenced inside its v1 document: panorama, character,
object, and capture records, including captures that were never sent back to
the canvas. Import rewrites the sidecar's root Creative Studio `projectId` and
all nested asset IDs before persisting it, while retaining internal
camera/entity/timeline identities. The outer Director pointer, known image and
video composer/mask `config.operation` node/asset references, graph identities, manifest checksums,
and content paths are remapped from the same complete identity maps.

The archive parser rejects missing dependencies, unknown reference-bearing
canvas operations, invalid Director envelopes, stale project ownership,
undeclared entries, checksum drift, and ZIP budget violations. Export,
project/asset deletion protection, and startup managed-data audit share the
same Director asset-closure resolver, so hidden scene dependencies cannot be
deleted or omitted independently of their project.
