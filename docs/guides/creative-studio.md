# Creation

Creation is NomiFun Desktop's focused, local-first creation product,
centered on:

- **Canvas**: a persistent infinite canvas with media nodes, auditable
  generation operations, reusable assets, and private templates;
- **Asset Library resources**: materials, the prompt library, and private
  templates, each reachable directly from the main sidebar.

Creation has no Project product object. A Canvas is a Canvas. Generation uses
NomiFun's existing provider and model catalog; Creation does not maintain a
second model configuration system. The former standalone Image and Video
Workbench pages are retired — generation tasks run through Canvas nodes and
template steps.

> Simplified Chinese: [creative-studio.zh.md](creative-studio.zh.md)

## Open the product

The main sidebar exposes Creation resources directly: **My Canvases**
(`creativeStudio.navigation.canvases`, `/nomi/canvases`), **Asset Library**
(`/asset-library/materials`), **Prompt Library** (`/asset-library/prompts`),
and **Template Studio** (`/asset-library/templates`) under the data section.
The My Canvases entry resumes the last valid product location in the current
app session, including its full query string and in-page hash. Invalid,
unknown, external, or overlong saved locations fail closed to
`/nomi/canvases`. Prompt-led creation is available from the **Canvas
Assistant** inside an opened Canvas.

The canonical route surface is:

| Route | Purpose |
| --- | --- |
| `/nomi/canvases` | Create, rename, open, import, export, and delete Canvases. |
| `/nomi/canvases/:canvasId` | Edit one Canvas's canonical infinite document. |
| `/asset-library/materials` | Manage reusable assets in My Assets. |
| `/asset-library/prompts` | Manage prompts in the Prompt Library. |
| `/asset-library/templates` | Manage private templates in Template Studio. |

The retired `/workshop/*` paths are no longer mounted as routes. A stored
resume location under `/workshop`, `/workshop/canvases`, `/workshop/projects`,
or `/workshop/canvas/:canvasId` is rewritten one-way by
`migratedCreativeRoute()` in
[`resourceRoutes.ts`](../../ui/src/renderer/pages/creativeStudio/app/resourceRoutes.ts)
to the corresponding `/nomi/canvases` destination. The standalone Image and
Video Workbench pages are retired; generation tasks run through Canvas nodes
and templates instead.

## Domain boundaries

The task owner union is intentionally small — the create-task API accepts
exactly two owner kinds, and the service additionally supports a
conversation-turn owner:

| Owner | Identity | Used by |
| --- | --- | --- |
| `CanvasNode` | `{ canvasId, nodeId }` | Tasks started from a Canvas node. |
| `TemplateStep` | `{ templateId, templateRunId, templateStepId }` | Template execution. |
| `ConversationTurn` | `{ conversationId, messageId }` | Creation tasks bound to a conversation turn. |

Only a task started by a Canvas node has a Canvas owner. The retired
`standalone_workbench` owner kind is rejected by the wire contract; historical
rows may retain a legacy `project_id` value as inert provenance, but it does
not participate in owner equality, history paging, retirement, asset origin
matching, or Canvas deletion.

Deleting a Canvas is blocked only by live `CanvasNode` tasks owned by that
Canvas.

## Canvas model

Each Canvas persists a versioned `nomifun.creative-studio/v1` document. Its
graph has exactly seven canonical node kinds:

| Node | Current role |
| --- | --- |
| `text` | Plain-text or Markdown content. |
| `image` | A real image asset, an empty image target, and its durable T2I/I2I composer draft. |
| `video` | A real video asset or an empty T2V/I2V target with a durable composer draft. |
| `audio` | A real audio asset or an empty TTS target with a durable composer draft. |
| `timeline` | A durable image/video editing timeline and its clip arrangement. |
| `config` | The auditable owner of an exact generation operation, parameters, task state, inputs, and results. |
| `group` | A container created by grouping an existing selection; it is not presented as a generator. |

Generator, loop, compare, and output are not canonical node kinds. Generation
is represented by media nodes plus `config`; graph grouping is an explicit
selection action.

The Canvas supports selection, movement, resize, connections, grouping,
copy/paste, undo/redo, zoom, reset/fit, minimap navigation, and reload on the
supported desktop shell and desktop WebUI viewport range (880x600 or larger).

Canvas edits use a short debounced compare-and-swap (CAS) save. Every write
sends the last authoritative revision. A conflict stops automatic saving; it
never force-writes or silently retries over a newer document. Resolve the
visible conflict by loading the authoritative remote version, then reapply the
intended change. Navigation out of Creation flushes pending Canvas
writes and remains blocked if their result is not safe.

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
wire compatibility, not a current Creation domain object.

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

A model selection is the exact pair `{ providerId, model }`. Creation
queries NomiFun's managed catalog for the required task and excludes disabled
providers, disabled models, and models that only advertise a neighbouring
task. It does not infer capability from a model name or silently substitute a
different task.

| Operation | Required NomiFun task | Creation capability |
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

## Retired standalone workbenches

The standalone Image and Video Workbench pages are retired and no longer
mounted. The create-task wire contract accepts only `canvas_node` and
`template_step` owners; a `standalone_workbench` owner is rejected. Historical
standalone rows remain readable as provenance but cannot be retried exactly.
Image, video, and audio generation now run through Canvas node composers and
template steps.

Canvas node composers keep durable drafts of prompt, exact
`{ providerId, model }` identity, controlled generation parameters, and ordered
reference asset IDs. On hydration, missing, unreadable, mismatched, duplicate,
excess, or wrong-kind references are removed rather than represented by stale
browser objects. Generation remains disabled until initial hydration finishes. A saved
model survives only if that exact Provider/model pair still supports the
required task; a same-named model from another Provider is never substituted.

## Prompt library and reusable inputs

`/asset-library/prompts` is a standalone prompt-management surface. It combines:

- an attributed, offline-first catalog synchronized from a fixed allow-list of
  upstream prompt repositories;
- enabled NomiFun presets that contain conversation instructions;
- user-owned text assets already stored in the Creation asset library.

The library supports text search, exact category filters, intersected tag
filters, detail inspection, and clipboard copy. A catalog or preset item can be
saved explicitly as a text asset in **My Assets**; catalog provenance keeps the
source repository and license attached. Prompt-saved assets carry a stable
source identity: they do not feed back into the prompt library as new entries,
and the same source is saved at most once. The details view can remove one from
**My Assets** without deleting its record or any Canvas reference, and it can
be added again later. Independently authored text assets remain valid prompt
sources. The catalog cache remains usable offline after a successful
synchronization.

The prompt-library route deliberately has no hidden Canvas insertion target.
Copying or saving a prompt does not create a Canvas or start generation. Pick
the resulting text asset from a Canvas when it belongs in a specific creation
flow.

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

Deleting an asset from **My Assets** permanently removes its original file and
thumbnail. Canvas nodes, completed generation tasks, and template history keep
an explicit “Asset deleted” marker; the original content cannot be restored by
reloading, replaying a task, or re-adding an old asset ID. Such markers are not
listed as reusable assets. An asset used by a running generation or template
run must wait until that work has finished or been canceled.

Content deletion is idempotent. SQLite retains pending cleanup paths until all
files have been removed, so a failed request can be retried and interrupted
cleanup resumes on the next startup. A missing file without an explicit
user-deletion marker still fails integrity checks. Existing external downloads
and backups are outside this deletion operation.

## Canvas archives

The canonical Canvas export is a version-3
`*.nomifun-canvas.zip` archive. Its manifest uses Canvas identity and carries
the validated Canvas document plus the complete referenced asset closure.
Import validates the archive and remaps Canvas, node, connection, asset,
operation, and session references so the imported copy does not alias the source. Version 3
preserves deleted-asset markers with empty content entries; import never
recreates the deleted media.

The reader also accepts version-2 Canvas archives and the released version-1
`.nomifun-canvas.zip` format. A v1 manifest may contain historical
`project/projectId` fields; those fields are compatibility wire data and do not
reintroduce a Project domain into the product. Conversation messages and active
pending turns live outside the archive, and import does not clone a
Conversation.

Archives do not contain provider credentials or install a missing provider or
model. Global templates and unrelated library assets are not implicitly added
to a Canvas archive.

## Minimal Template AI

**AI Create** on `/asset-library/templates` intentionally implements a small launch
scope:

1. Enter a simple requirement and select one exact enabled `chat` model.
2. NomiFun performs one tool-less completion with a 120-second wall-clock
   budget, a 4,096-token output ceiling, and a 262,156-byte local response cap.
3. The client accepts only one strict final
   `nomifun.creative-studio.template-draft/v1` JSON artifact. The available
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
Apply all normalize the underlying template definition to `private`, and there is no
public-visibility control.

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
- The default titlebar and Creation rail follow the application locale,
  but much of the launch Canvas and editor body remains Simplified Chinese.
- A configured model is not evidence that its remote provider is reachable or
  that a paid request was executed. Keep provider billing and data policies in
  mind before generating.
- Responsive browser layouts do not establish complete touch-device support.

## Reading verification claims

Creation validation is reported in layers so one result is not mistaken
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

- Product routes: [`app/resourceRoutes.ts`](../../ui/src/renderer/pages/creativeStudio/app/resourceRoutes.ts)
- Canvas document: [`creative_studio.rs`](../../crates/backend/nomifun-workshop/src/creative_studio.rs)
- Canvas, asset, and template routes: [`nomifun-workshop/src/routes.rs`](../../crates/backend/nomifun-workshop/src/routes.rs)
- Generation task routes: [`nomifun-creation/src/routes.rs`](../../crates/backend/nomifun-creation/src/routes.rs)
- Model selection: [`models/catalog.ts`](../../ui/src/renderer/pages/creativeStudio/models/catalog.ts)
