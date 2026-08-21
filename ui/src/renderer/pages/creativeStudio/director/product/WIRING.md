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

Project ZIP export/import includes the Director `sceneId` text sidecar and
remaps the pointer on import. Archive tests cover collection, checksum,
identity remapping and round-trip recovery of that scene document.
