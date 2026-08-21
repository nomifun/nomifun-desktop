# Creative Director product route wiring

The default export is a no-props route component. It reads the canonical
`projectId` with `useParams`, loads the root Creative Studio project through
`creativeProjectRepository`, and saves its Director scene through revision CAS.

Add the lazy route import beside the other Creative Studio products:

```tsx
const CreativeDirectorProductRoute = React.lazy(
  () => import("@renderer/pages/creativeStudio/director/product"),
);
```

Mount it inside `CreativeStudioFocusShell`:

```tsx
<Route
  path="director/:projectId"
  element={withRouteFallback(CreativeDirectorProductRoute)}
/>
```

The root canvas document remains authoritative. Its single `director` node
stores a `sceneId` pointer to an immutable, non-library NomiFun text asset whose
content is the validated `nomifun.director.project` v1 document. Every pointer
advance uses the root project's compare-and-swap revision; conflicts never
force-write.

The product registers an asynchronous leave gate. Focus-shell navigation must
also await `requestCreativeDirectorProductBeforeLeave()` and continue only when
it returns `true`, just as it already does for the canvas product.
The Director close action awaits the same gate and returns to
`creativeStudioCanvasProjectPath(projectId)`, preserving the current project's
canvas context instead of dropping the user at the project index.

The current workshop asset backend accepts image, video and audio uploads but
not GLB/glTF. The route therefore keeps model/character import explicitly
unavailable instead of substituting unlicensed or fake geometry. Single-frame
Three.js screenshots are uploaded through the real NomiFun asset client;
multi-angle capture, video export and send-to-canvas remain visibly unwired.

The current backend archive collector also ignores `director.sceneId`; route
wiring alone therefore does not make Director sidecars portable. Project ZIP
export/import must not be declared complete for Director scenes until the
backend archive collector and remapper explicitly include this pointer.
