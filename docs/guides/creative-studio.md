# Creation

Creation is NomiFun Desktop's focused, local-first creation product,
centered on:

- **Canvas**: a persistent infinite canvas with media nodes, auditable
  generation operations, reusable assets, and private templates;
- **Asset Library resources**: materials, the prompt library, and private
  templates, each reachable directly from the main sidebar.

Generation uses NomiFun's existing provider and model catalog. Configure models
in Model Management, then select them from Canvas nodes and template steps.

> Simplified Chinese: [creative-studio.zh.md](creative-studio.zh.md)

## Open the product

The main sidebar exposes Creation resources directly: **My Canvases**
(`/nomi/canvases`), **Asset Library**
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

## Make a first creation

1. Open **My Canvases**, create a Canvas and add a text or media node.
2. For generation, choose the matching image, video or audio operation and a configured model supporting that task. Add a prompt and reference assets where required.
3. Submit once and follow the task state on the Canvas. Reloading uses the saved task state; a network error is not proof the provider did nothing.
4. Inspect the result, save useful media to My Assets and reuse it in other nodes or templates.
5. Canvas Assistant proposals remain proposals until you choose **Apply to Canvas**. Review the proposed graph before applying.

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

The asset library supports text, image, video and audio assets, with search, type filters, collections, tags and reusable pickers. Binary uploads are limited to 64 MiB.

Submitted work and generation settings persist with the Canvas. After a reload, task state is reconciled with the backend. When a response is uncertain, inspect that task before submitting again.

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

Export a Canvas as `*.nomifun-canvas.zip` to carry its saved document and referenced assets. Import creates independent identities; deleted-asset markers remain deleted. Conversation messages and active turns are not cloned.

Archives do not contain provider credentials, install missing models, or include unrelated templates and assets. Reconfigure the required providers on the destination installation.


## Create a template with AI

1. Open **AI Create** in the template workbench and select an enabled Chat model.
2. Describe a single-image or multi-image-series template.
3. Review the returned preview. **Apply** opens a private draft in the template editor.
4. Edit and select **Save** to persist it. Applying alone does not save or run it.

The draft request does not use tools or automatically retry, switch models, save or generate media. Templates remain private; public publishing and discovery are unavailable.


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
  but much of the Canvas and editor body remains Simplified Chinese.
- A configured model is not evidence that its remote provider is reachable or
  that a paid request was executed. Keep provider billing and data policies in
  mind before generating.
- Responsive browser layouts do not establish complete touch-device support.
