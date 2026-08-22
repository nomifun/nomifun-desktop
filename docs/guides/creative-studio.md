# Creative Studio

Creative Studio is NomiFun Desktop's focused, local-first creation product. It
combines a persistent infinite canvas, project-scoped generation, standalone
image and video workbenches, reusable assets and prompts, minimal workflows,
and a deliberately bounded 3D Director. It uses NomiFun's existing provider and
model catalog; it does not maintain a second model configuration system.

> Simplified Chinese: [creative-studio.zh.md](creative-studio.zh.md)

> **Transitional domain note (2026-08-23):** the approved product direction
> removes Project from Creative Studio. Canvases, Image Workbench, and Video
> Workbench are independent products. References below to `project/projectId`
> and project-scoped standalone workbenches describe the current compatibility
> implementation, not the target contract. See the linked Chinese redesign spec.

## Open the product

Open **Creative Studio** from the application sidebar. The page reuses
NomiFun's default titlebar controls for the sidebar, history, and system window.
Like Settings, the primary rail switches to Creative Studio navigation and can
be collapsed to recover working space. The primary Creative Studio entry resumes
the last valid product location in the current app session, including its full
query string and in-page hash. Use the **Creative Studio** home item in the
product rail when you intentionally want a fresh
starting point. **Back to Workbench** stays pinned to the bottom of that rail and
returns to `/guid` after any pending canvas or Director save has been resolved.

The active route surface is:

| Route | Purpose |
| --- | --- |
| `/workshop` | Start a project, optionally with a simple exact-Chat kickoff. |
| `/workshop/projects` | Create, rename, open, import, export, and delete projects. |
| `/workshop/canvas/:projectId` | Edit one project's canonical infinite canvas. |
| `/workshop/director/:projectId` | Edit the same project's bounded 3D Director scene. |
| `/workshop/image`, `/workshop/video` | Run project-owned standalone image or video tasks. |
| `/workshop/prompts`, `/workshop/assets`, `/workshop/workflows` | Manage prompts, reusable assets, and private workflows. |

`/workshop/audio` is retired and is not an active route. Audio creation remains
available through audio nodes on the project canvas.

## Canvas model

Each project persists a versioned `nomifun.creative-studio/v1` document. Its
graph has exactly eight canonical node kinds:

| Node | Current role |
| --- | --- |
| `text` | Plain-text or Markdown content. |
| `image` | A real image asset, an empty image target, and its durable T2I/I2I composer draft. |
| `video` | A real video asset or an empty T2V/I2V target with a durable composer draft. |
| `audio` | A real audio asset or an empty TTS target with a durable composer draft. |
| `panorama` | A real equirectangular panorama asset and view state. |
| `config` | The auditable owner of an exact generation operation, parameters, task state, inputs, and results. |
| `director` | A pointer from the canvas to project Director scene/camera/timeline state. |
| `group` | A container created by grouping an existing selection; it is not presented as a generator. |

Generator, loop, compare, and output are not canonical node kinds. Generation
is represented by media nodes plus `config`; graph grouping is an explicit
selection action.

The canvas supports selection, movement, resize, connections, grouping,
copy/paste, undo/redo, zoom, reset/fit, minimap navigation, and project reload.
Narrow layouts are supported, but this is not a claim of complete mobile touch
or gesture parity.

## Exact model and task routing

A model selection is the exact pair `{ providerId, model }`. Creative Studio
queries NomiFun's managed catalog for the required task and excludes disabled
providers, disabled models, and models that only advertise a neighbouring
task. It does not infer capability from a model name or silently substitute a
different task.

| Operation | Required NomiFun task | Creative Studio capability |
| --- | --- | --- |
| Simple kickoff and Canvas assistant | `chat` | project-bound assistant turn; strict graph proposals still require manual approval |
| Workflow AI draft/planning | `chat` | one tool-less, bounded completion |
| Empty image target | `image_generation` | `t2i` |
| Image with real references, including the current mask-edit path | `image_edit` | `i2i` |
| Empty video target | `video_generation` | `t2v` |
| Video with exactly one direct real image reference | `video_generation` | `i2v` |
| Empty audio target | `speech_synthesis` | `tts` |

The stored operation keeps the provider, model, task, capability, ordered input
asset bindings, and typed parameters together. A retry cannot change those
facts while reusing the same idempotency identity. Removing a provider or model
also goes through coordinated checks so active tasks and other hard bindings
cannot be orphaned silently.

### Governed Gateway access

The in-process Gateway exposes the same domain through six instance-owner
capabilities: `nomi_creative_studio_list_projects`,
`nomi_creative_studio_get_project`, `nomi_creative_studio_list_assets`,
`nomi_creative_studio_apply_ops`, `nomi_creative_studio_generate`, and
`nomi_creative_studio_get_task`. They read the same canonical documents and
assets, use the same project revision CAS, and submit through the same
idempotent task queue as the UI. They are visible only to the curated `desktop`
and `admin` Gateway profiles; `work` and `lite` profiles, ordinary
conversations, companions, and non-owner callers cannot discover or invoke
them.

## Persistence, conflicts, and recovery

Project documents live in SQLite. Asset metadata also lives in SQLite, while
binary originals and thumbnails live under the backend data directory's
`workshop/assets/` tree.

Canvas edits use a short debounced compare-and-swap (CAS) save. Every write
sends the last authoritative revision. A conflict stops automatic saving; it
never force-writes or silently retries over a newer document. Resolve the
visible conflict by loading the authoritative remote version, then reapply the
intended change. Navigation out of Creative Studio flushes pending canvas
or Director writes and remains blocked if their result is not safe.

Image, video, and audio composer drafts are stored on their owning nodes.
Submitted work also has one durable `config` owner and one canonical creation
task. After reload, the UI reconciles only that exact owner with authoritative
task state. Terminal settlement is idempotent, and an uncertain response does
not invent success or discard the audit trail. A confirmed `404` is treated
differently from a transient network failure.

Canvas Agent changes are proposals, not background mutations. The supported
proposal artifact is parsed fail-closed and only a user's **Apply to canvas**
action performs the project CAS write. Delete and media generation are outside
that proposal subset.

## Projects, ZIP archives, and assets

Project Center exports each selected project as its own
`*.nomifun-canvas.zip`. The archive contains the validated project document and
the full referenced asset closure, including the Director sidecar and its
referenced assets. Import validates the archive, creates a new project, and
remaps project, node, connection, asset, operation, chat/session, and Director
references so the imported copy does not alias the source project.

Conversation messages and active pending turns live outside the project
archive. Import clears those external references while preserving only safe
project-owned state; it does not clone a Conversation.

Archives do not contain provider credentials or install a missing provider or
model. Global workflows and unrelated library assets are not implicitly added
to a project archive.

The asset library supports real `text`, `image`, `video`, and `audio` assets,
with search, kind filters, collections, tags, metadata updates, and reusable
pickers. Binary upload is capped at 64 MiB. All list and mutation APIs are
instance-owner-only. `GET /api/creative-studio/files/{assetId}` is the narrow
read-only exception: browser media elements cannot attach the desktop trust
header, so an opaque UUIDv7 acts as the capability URL. It is not a listing or
write surface.

## Minimal Workflow AI

**AI Create** on `/workshop/workflows` intentionally implements a small launch
scope:

1. Enter a simple requirement and select one exact enabled `chat` model.
2. NomiFun performs one tool-less completion with a 120-second wall-clock
   budget, a 4,096-token output ceiling, and a 262,156-byte local response cap.
3. The client accepts only one strict final
   `nomifun.creative-studio.workflow-draft/v1` JSON artifact. The available
   draft modes are `single-image` and `multi-image-series`.
4. Review the preview. **Apply** only opens the existing workflow editor with a
   private in-memory draft.
5. Edit it as needed and click **Save**. Only this explicit Save creates the
   workflow. Apply does not persist or run it.

The one-shot request creates no Conversation, attachment, public template,
Skill/MCP tool session, workflow, or workflow run. There is no automatic retry,
model failover, save, or execution. The model never chooses IDs, revisions,
timestamps, visibility, tags, media-generation models, or assets. Public
template publishing/discovery and complex Workflow conversations are not part
of this launch scope. The launch UI is private-only: create, edit, copy, and AI
Apply all normalize the workflow to `private`, and there is no public-visibility
control.

## Director v1 subset

Director is a project-bound Three.js scene editor, not a full DCC or video
editor. The current product supports scene and camera transforms, camera aspect
and thirds guides, a real 2:1 panorama environment, timeline duration/playback/
loop, camera-position tracks and keyframes, current-camera PNG/JPEG capture,
uploading captures as NomiFun image assets, and idempotently sending those
captures back to the canvas. Director state is stored as a versioned text
sidecar referenced by the project document and advanced through project CAS.

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
  but much of the launch canvas and editor body remains Simplified Chinese.
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
- Canonical document: [`creative_studio.rs`](../../crates/backend/nomifun-workshop/src/creative_studio.rs)
- Project/asset/workflow routes: [`nomifun-workshop/src/routes.rs`](../../crates/backend/nomifun-workshop/src/routes.rs)
- Generation task routes: [`nomifun-creation/src/routes.rs`](../../crates/backend/nomifun-creation/src/routes.rs)
- Model selection: [`models/catalog.ts`](../../ui/src/renderer/pages/creativeStudio/models/catalog.ts)
