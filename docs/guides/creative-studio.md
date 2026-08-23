# Creative Studio

Creative Studio is NomiFun Desktop's focused, local-first creation product.
It has three independent creation surfaces:

- **Canvas**: a persistent infinite canvas with media nodes, auditable
  generation operations, reusable assets, private templates, and a bounded
  Director.
- **Image Workbench**: a standalone image-generation workbench.
- **Video Workbench**: a standalone video-generation workbench.

Creative Studio has no Project product object. A Canvas is a Canvas. Image and
Video Workbenches do not require, infer, select, or create a Canvas. They use
NomiFun's existing provider and model catalog; they do not maintain a second
model configuration system.

> Simplified Chinese: [creative-studio.zh.md](creative-studio.zh.md)

## Open the product

Open **Creative Studio** from the application sidebar. The page reuses
NomiFun's default titlebar controls for the sidebar, history, and system
window. Like Settings, the primary rail switches to Creative Studio
navigation and can be collapsed to recover working space. The primary
Creative Studio entry resumes the last valid product location in the current
app session, including its full query string and in-page hash. Invalid,
unknown, external, or overlong saved locations fail closed to
`/workshop/canvases`. There is no separate Creative Studio home item; prompt-led
creation is available from the **Canvas Assistant** inside an opened Canvas.
**Back to Workbench** stays pinned to the bottom of that rail and returns to
`/guid` after any pending Canvas or Director save has been resolved.

The canonical route surface is:

| Route | Purpose |
| --- | --- |
| `/workshop` | Compatibility entry that redirects to `/workshop/canvases`. |
| `/workshop/canvases` | Create, rename, open, import, export, and delete Canvases. |
| `/workshop/canvas/:canvasId` | Edit one Canvas's canonical infinite document. |
| `/workshop/director/:canvasId` | Edit the bounded Director state attached to that Canvas. |
| `/workshop/image` | Use the standalone Image Workbench. It is fully usable with zero Canvases. |
| `/workshop/video` | Use the standalone Video Workbench. It is fully usable with zero Canvases. |
| `/workshop/prompts`, `/workshop/assets`, `/workshop/templates` | Manage prompts, reusable assets, and private templates in Template Studio. |

`/workshop/projects` is a deprecated compatibility route that redirects to
`/workshop/canvases`. It is not a product surface or a second name for a
Canvas. `/workshop/audio` is retired; audio creation remains available through
audio nodes on a Canvas.

## Domain boundaries

The task owner union is intentionally small:

| Owner | Identity | Used by |
| --- | --- | --- |
| `CanvasNode` | `{ canvasId, nodeId }` | Tasks started from a Canvas node. |
| `StandaloneWorkbench` | `{ workbenchKind }` | Image, Video, or other standalone workbench tasks. |
| `WorkflowStep` | Existing workflow/run/step identity | Workflow execution. |

Only a task started by a Canvas node has a Canvas owner. A standalone task
never gets a hidden, temporary, default, or automatically selected Canvas.
Historical standalone rows may retain a legacy `project_id` value as inert
provenance, but it does not participate in owner equality, history paging,
retirement, asset origin matching, or Canvas deletion. New standalone tasks
write no legacy project binding, and new standalone asset origins carry only
`workbench_kind`.

Deleting a Canvas is blocked only by live `CanvasNode` tasks owned by that
Canvas. Live standalone tasks do not block deletion of any Canvas.

## Canvas model

Each Canvas persists a versioned `nomifun.creative-studio/v1` document. Its
graph has exactly eight canonical node kinds:

| Node | Current role |
| --- | --- |
| `text` | Plain-text or Markdown content. |
| `image` | A real image asset, an empty image target, and its durable T2I/I2I composer draft. |
| `video` | A real video asset or an empty T2V/I2V target with a durable composer draft. |
| `audio` | A real audio asset or an empty TTS target with a durable composer draft. |
| `panorama` | A real equirectangular panorama asset and view state. |
| `config` | The auditable owner of an exact generation operation, parameters, task state, inputs, and results. |
| `director` | A pointer from the Canvas to its Director scene, camera, and timeline state. |
| `group` | A container created by grouping an existing selection; it is not presented as a generator. |

Generator, loop, compare, and output are not canonical node kinds. Generation
is represented by media nodes plus `config`; graph grouping is an explicit
selection action.

The Canvas supports selection, movement, resize, connections, grouping,
copy/paste, undo/redo, zoom, reset/fit, minimap navigation, and reload.
Narrow layouts are supported, but this is not a claim of complete mobile touch
or gesture parity.

Canvas edits use a short debounced compare-and-swap (CAS) save. Every write
sends the last authoritative revision. A conflict stops automatic saving; it
never force-writes or silently retries over a newer document. Resolve the
visible conflict by loading the authoritative remote version, then reapply the
intended change. Navigation out of Creative Studio flushes pending Canvas or
Director writes and remains blocked if their result is not safe.

Canvas Agent changes are proposals, not background mutations. The supported
proposal artifact is parsed fail-closed and only a user's **Apply to Canvas**
action performs the Canvas CAS write. Delete and media generation are outside
that proposal subset.

## Canvas API and Gateway

The canonical HTTP resource is:

- `GET/POST /api/creative-studio/canvases`
- `GET/PATCH/DELETE /api/creative-studio/canvases/:canvasId`
- `PUT /api/creative-studio/canvases/:canvasId/document`
- Canvas agent operations and archive actions are rooted at the same Canvas
  resource.

The old `/api/creative-studio/projects` routes remain only as deprecated
compatibility aliases. Their legacy `project/projectId` names describe old
wire compatibility, not a current Creative Studio domain object.

The in-process Gateway exposes the Canvas-first capabilities
`nomi_creative_studio_list_canvases` and
`nomi_creative_studio_get_canvas`, alongside asset, apply-ops, generation, and
task capabilities. The old `nomi_creative_studio_list_projects` and
`nomi_creative_studio_get_project` capabilities are deprecated legacy aliases.
All of these are instance-owner capabilities visible only to the curated
`desktop` and `admin` Gateway profiles; `work` and `lite` profiles, ordinary
conversations, companions, and non-owner callers cannot discover or invoke
them.

The UI/API contract version for this wire transition is **21**.

## Exact model and task routing

A model selection is the exact pair `{ providerId, model }`. Creative Studio
queries NomiFun's managed catalog for the required task and excludes disabled
providers, disabled models, and models that only advertise a neighbouring
task. It does not infer capability from a model name or silently substitute a
different task.

| Operation | Required NomiFun task | Creative Studio capability |
| --- | --- | --- |
| Canvas Assistant | `chat` | Canvas-scoped assistant turn; strict graph proposals still require manual approval. |
| Template AI draft/planning | `chat` | One tool-less, bounded completion. |
| Empty image target | `image_generation` | `t2i`. |
| Image with real references, including the mask-edit path | `image_edit` | `i2i`. |
| Empty video target | `video_generation` | `t2v`. |
| Video with exactly one direct real image reference | `video_generation` | `i2v`. |
| Empty audio target on a Canvas | `speech_synthesis` | `tts`. |

The stored operation keeps the provider, model, task, capability, ordered input
asset bindings, and typed parameters together. A retry cannot change those
facts while reusing the same idempotency identity. Removing a provider or model
also goes through coordinated checks so active tasks and other hard bindings
cannot be orphaned silently.

## Standalone Image and Video Workbenches

Image and Video are independent workbenches. Their routes have no Canvas query,
selector, parent-load gate, or scope bar, and they remain fully usable when the
Canvas list is empty. They never create or select a hidden Canvas.

Standalone task history is scoped only by `workbench_kind`:

- `GET /api/creative-studio/tasks?workbench_kind=image|video`
- `POST /api/creative-studio/tasks/retire` with
  `{ workbench_kind, task_ids }`

History pagination, active-task recovery, retry, retirement, and asset-origin
matching ignore legacy standalone `project_id` provenance. The history model
merges old provenance buckets by task identity and retains strict keyset
pagination. Legacy rows whose ordered inputs cannot be proven remain visible but
cannot be retried exactly.

### Session draft continuity

Image and Video keep one versioned `sessionStorage` draft per `workbenchKind`.
The storage key contains neither `projectId` nor `canvasId`. A draft stores only:

- prompt;
- exact `{ providerId, model }` identity;
- controlled generation parameters;
- ordered reference asset IDs;
- the workbench layout.

Busy state, errors, open modals, current selections, task state, and complete
asset objects are deliberately excluded. Corrupt, oversized, unknown-version,
cross-workbench, or unavailable-storage values fail closed without preventing
the route from loading.

On route entry, reference IDs are hydrated individually through the canonical
asset `get` API. Missing, unreadable, mismatched, duplicate, excess, or
wrong-kind references are removed rather than represented by stale browser
objects. Generation remains disabled until initial hydration finishes. A saved
model survives only if that exact Provider/model pair still supports the
required task; a same-named model from another Provider is never substituted.

## Assets, persistence, and recovery

Asset metadata lives in SQLite, while binary originals and thumbnails live under
the backend data directory's `workshop/assets/` tree. The asset library
supports real `text`, `image`, `video`, and `audio` assets, with search, kind
filters, collections, tags, metadata updates, and reusable pickers. Binary
upload is capped at 64 MiB. All list and mutation APIs are instance-owner-only.
`GET /api/creative-studio/files/{assetId}` is the narrow read-only exception:
browser media elements cannot attach the desktop trust header, so an opaque
UUIDv7 acts as the capability URL. It is not a listing or write surface.

Submitted Canvas work has one durable `config` owner and one canonical creation
task. After reload, the UI reconciles only that exact owner with authoritative
task state. Terminal settlement is idempotent, and an uncertain response does
not invent success or discard the audit trail. A confirmed `404` is treated
differently from a transient network failure.

## Canvas archives

The canonical Canvas export is a version-2
`*.nomifun-canvas.zip` archive. Its manifest uses Canvas identity and carries
the validated Canvas document plus the complete referenced asset closure,
including the Director sidecar and its referenced assets. Import validates the
archive and remaps Canvas, node, connection, asset, operation, session, and
Director references so the imported copy does not alias the source.

The reader must continue to accept the released version-1
`.nomifun-canvas.zip` format. A v1 manifest may contain historical
`project/projectId` fields; those fields are compatibility wire data and do not
reintroduce a Project domain into the product. Conversation messages and active
pending turns live outside the archive, and import does not clone a
Conversation.

Archives do not contain provider credentials or install a missing provider or
model. Global templates and unrelated library assets are not implicitly added
to a Canvas archive.

## Minimal Template AI

**AI Create** on `/workshop/templates` intentionally implements a small launch
scope:

1. Enter a simple requirement and select one exact enabled `chat` model.
2. NomiFun performs one tool-less completion with a 120-second wall-clock
   budget, a 4,096-token output ceiling, and a 262,156-byte local response cap.
3. The client accepts only one strict final
   `nomifun.creative-studio.workflow-draft/v1` JSON artifact. The available
   draft modes are `single-image` and `multi-image-series`.
4. Review the preview. **Apply** only opens the existing template editor with a
   private in-memory draft.
5. Edit it as needed and click **Save**. Only this explicit Save creates the
   asset template. Apply does not persist or run it.

The one-shot request creates no Conversation, attachment, published template,
Skill/MCP tool session, saved template, or template run. There is no automatic retry,
model failover, save, or execution. The model never chooses IDs, revisions,
timestamps, visibility, tags, media-generation models, or assets. Public
template publishing/discovery and complex template conversations are not part
of this launch scope. The launch UI is private-only: create, edit, copy, and AI
Apply all normalize the underlying workflow definition to `private`, and there is no
public-visibility control.

## Director v1 subset

Director is a Canvas-bound Three.js scene editor, not a full DCC or video
editor. The current product supports scene and camera transforms, camera
aspect and thirds guides, a real 2:1 panorama environment, timeline
duration/playback/loop, camera-position tracks and keyframes, current-camera
PNG/JPEG capture, uploading captures as NomiFun image assets, and idempotently
sending those captures back to the Canvas. Director state is stored as a
versioned text sidecar referenced by the Canvas document and advanced through
Canvas CAS.

The current asset backend does not accept GLB/glTF model imports, so character
and model-library actions do not create placeholders. Four-view and twelve-view
batch capture, timeline/video export, and full panorama/video production remain
unavailable.

## Current limits

- Video currently supports T2V and one-image I2V. V2V, first/last-frame input,
  multiple image references, mixed video/audio references, and untyped hidden
  provider parameters are rejected.
- Canvas audio generation currently supports zero-input TTS to one MP3 or WAV
  result. Reference audio, voice cloning, audio-to-audio, speed/instructions,
  AAC, and PCM are not exposed by this contract.
- Provider protocols differ. Controls appear only when their exact typed
  protocol profile supports them; unknown protocols use the smaller safe
  subset.
- The default titlebar and Creative Studio rail follow the application locale,
  but much of the launch Canvas and editor body remains Simplified Chinese.
- A configured model is not evidence that its remote provider is reachable or
  that a paid request was executed. Keep provider billing and data policies in
  mind before generating.
- Responsive browser layouts do not establish complete touch-device support.

## Reading verification claims

Creative Studio validation is reported in layers so one result is not mistaken
for another:

1. **Contract checks** — TypeScript/Rust tests, schema checks, type checking,
   theme/icon/dead-CSS checks, and compilation prove code-level contracts.
2. **Browser product checks** — real clicks, reloads, persistence counts,
   console inspection, and target viewports prove the exercised Web UI path.
   A local mock provider can close this path without spending provider credit.
3. **Host and artifact checks** — Web and Tauri slow loops, production UI
   builds, and platform packaging prove their specific host or artifact. They
   do not prove another operating system.
4. **Real-provider checks** — only an explicitly authorized request to the
   selected provider proves live credentials, vendor compatibility, latency,
   billing, and generated media quality.
5. **Release checks** — producing an installer is separate from code signing,
   notarization, updater verification, and publishing it to a release channel.

Unless a release record says otherwise, do not infer a paid-provider smoke test
or signed/published desktop release from source, unit, browser-mock, build, or
packaging success alone.

## Implementation references

- Product routes: [`app/routes.ts`](../../ui/src/renderer/pages/creativeStudio/app/routes.ts)
- Canvas document: [`creative_studio.rs`](../../crates/backend/nomifun-workshop/src/creative_studio.rs)
- Canvas, asset, and template routes: [`nomifun-workshop/src/routes.rs`](../../crates/backend/nomifun-workshop/src/routes.rs)
- Generation task routes: [`nomifun-creation/src/routes.rs`](../../crates/backend/nomifun-creation/src/routes.rs)
- Model selection: [`models/catalog.ts`](../../ui/src/renderer/pages/creativeStudio/models/catalog.ts)
