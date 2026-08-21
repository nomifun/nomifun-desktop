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
The product chrome is the single visible save/recovery surface and therefore
sets the generic Editor save banner off. It shows a stable Chinese conflict
message without leaking project IDs or backend diagnostics; only the backend
code `REVISION_CONFLICT` enables “重新载入远端”. A generic business 409 remains
an ordinary save error and does not invite the user to discard local work.

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
`../editor/WIRING.md`. Project-scoped image, video, and audio task runtimes are
mounted only after the project and Editor graph are both hydrated. The image
runtime routes two strict persisted operations without creating a second
document controller:

- `image-mask-edit` uploads the blue-marked reference as a hidden real asset.
- `image-node-compose` submits empty image nodes as exact
  `image_generation` / `t2i`, while an image with a real asset submits exact
  `image_edit` / `i2i` with that asset as the implicit reference.

Both operations persist a locked config owner plus `pendingTaskIds` before POST
and submit the exact task/capability/provider/model identity through the shared
NomiFun task client. Queued/running state is reconciled without creating
undo history; terminal result IDs are resolved to real image assets and written
as config-to-image nodes before pending removal performs the final CAS flush.
Mount recovery accepts only either exact operation marker. A
transport-ambiguous create keeps the same config and idempotency key for safe
retry; only an authoritative 404 may clear an orphaned pending reference.

The video runtime owns only `video-node-compose`. It accepts an empty video
node as exact `video_generation` / `t2v`, or the same empty node with exactly
one directly connected real image as `i2v`. It maps 720p/1080p and the supported
aspect ratios to concrete width/height, fixes repeat to one, keeps canvas owner
identity in `config.data.operation`, and never forwards local metadata through
provider parameters. V2V, multiple/first-last-frame references, audio/video
references, and provider-specific camera controls stay explicitly unavailable.

The audio runtime owns only `audio-node-compose`. Its first deliverable accepts
an empty audio node, no input assets, exact `speech_synthesis` / `tts`, and one
real audio result. Exact adapter protocol profiles decide whether Voice ID and
MP3/WAV format controls are exposed, whether voice is required, and the text
limit; unknown protocols receive prompt only. Speed, instructions, reference
audio, VoiceClone, AAC, and PCM are never sent by this slice. Successful
settlement fills the same audio node ID, clears only the now-inapplicable draft
model, and removes pending last. Failed/canceled configs remain auditable, and
ambiguous submission offers same-key retry plus an explicit status check that
cleans only an authoritative 404 orphan.

All three inline media composers own node-persisted drafts, use fixed light
stone creation palettes independent from the application theme, and switch to
a viewport portal when the canvas column cannot contain them.

The inline composer opens only for one selected image. A successful empty-node
`t2i` task idempotently fills that source node with the first real result; any
additional results become config-linked image nodes. If the empty source gains
a different asset before completion, settlement fails closed instead of
overwriting it. Existing-image `i2i` always writes new config-linked results and
never mutates the source asset.

The same single-selection boundary owns the reference-style image toolbar.
Empty images expose information, delete, and a real image-file chooser; filled
images expose those base actions plus the implemented image tools. Information
reuses the canonical properties panel, delete removes only the node/edges, and
upload preserves the node identity while updating it through `node/update`.
Uploads use a unique operation tag so a committed asset can be recovered after
response loss, re-read the latest Editor state before filling, and immediately
flush the full document CAS. A stale/deleted/already-filled node is never
overwritten; the real uploaded asset remains available in the library. The
toolbar uses a fixed dark focused palette and becomes a viewport overlay when a
narrow canvas column cannot contain it.
