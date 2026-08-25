# Creative Canvas product route wiring

This directory exports a default, no-props route component. It reads the
canonical `canvasId` through `useParams` and keeps `CreativeCanvasEditor` as
the only reducer and CAS persistence owner.

The production router mounts this default export at
`/workshop/canvas/:canvasId` inside `CreativeStudioFocusShell`.
It keeps the route split with
`import('@renderer/pages/creativeStudio/canvas/product')` and the nested
`path="canvas/:canvasId"` contract.

The product's own “返回项目” action awaits the editor CAS `flush()` and only
The product returns to the Canvas library at `/workshop/canvases` after
`noop` or `saved`. A
`conflict` or `error` stays on the canvas and exposes explicit reload/retry.
The product chrome is the single visible save/recovery surface and therefore
sets the generic Editor save banner off. It shows a stable Chinese conflict
message without leaking Canvas IDs or backend diagnostics; only the backend
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
an active exclusive Agent turn, then flushes the Editor. The left template panel
uses the canonical template repository and durable run controller, opens the
same typed runner and real asset picker as the standalone center, and resolves
successful result IDs through the authenticated asset-detail endpoint before
inserting canonical nodes. The bottom timeline now projects the Canvas's one
canonical Director node without inventing global tracks or keyframes. It shows
the saved scene pointer, camera pointer and timeline/duration values, and opens
the real Director product only after the canvas CAS leave gate succeeds. New UI
creation paths enforce one Director node per Canvas; malformed documents with
multiple Director nodes remain visible as a fail-closed conflict. The Director
close action returns to this same Canvas.

The authenticated Agent mutation gateway is
`POST /api/creative-studio/canvases/{canvasId}/agent-ops`. Its wrapper accepts
only `{ expectedRevision, ops }`; the audit source is server-owned and fixed to
`creative-studio-agent`. Nested operations reuse the one canonical snake_case
`CreativeAgentOp` wire contract, so the HTTP path, Gateway, service and domain do
not drift through duplicate DTOs. One request is one Canvas revision CAS: the
server mints node/connection UUIDv7 values, validates the complete resulting
Canvas document, and returns only the saved Canvas summary plus ordered op results.
A stale revision, unknown field, empty/invalid batch, runtime-owned task-field
patch or late invalid op produces zero writes. `delete_node` is additionally
rejected at this Agent route because deletion remains an explicit user-confirmed
canvas action. The product will invoke this gateway only after presenting a
strict planning artifact for manual “应用到画布” approval, then reload the
authoritative Canvas rather than mutating a second optimistic graph.

The Agent route is Canvas-owned: every mutation is bound to `canvasId`, and
node-scoped work uses `CanvasNode { canvasId, nodeId }`. The wire owner is
`{ kind: 'canvas_node', canvas_id, node_id }`; an owner without `canvas_id` is
invalid.

Agent pending turns now persist both the user-facing `prompt` and the exact
`modelInput` envelope plus ordered `skillIds`. Reload and response-loss recovery
therefore replay the same model input and Skill snapshot instead of rebuilding
them from whatever canvas selection happens to exist later. Coordinated v1
readers accept an older pending turn with no new fields and normalize it to
`modelInput = prompt` / `skillIds = []`; every new frontend serialization writes
the complete shape. Model input is trimmed/non-empty and bounded to 262144
UTF-16 units. Skill IDs are an ordered unique ASCII set of at most eight items,
each no longer than 128 units.

The pure canvas-context builder includes selected nodes first, then only their
one-hop graph, group and operation references, with at most 32 nodes and 64
relevant connections. Text/prompt fields are capped at 2000 Unicode characters,
data/blob payloads are removed, and neither resolved media URLs nor opaque
Provider parameters enter the envelope. The v1 planning envelope names the
allowed canvas-op artifact, requires explicit user approval and forbids node
deletion or media generation.

The Canvas Agent composer now consumes that contract. Current context nodes are
shown as removable chips; the user explicitly selects one to three packaged
NomiFun Skills (`creative-studio-canvas`, `creative-studio-organize`, and
`creative-studio-template`) instead of triggering prompt-regex pseudo-skills.
Submission builds the bounded envelope once, persists it with the ordered Skill
IDs, then the real Conversation transport sends `modelInput` and copies those
IDs to `inject_skills`. The visible user message and session title continue to
use the original prompt. Recovery reads only the durable pending snapshot, so a
later selection change cannot alter an admitted turn. No direct Provider/API-key
path or automatic model invocation was added.

Completed assistant messages now expose a canvas proposal only when their final
and unique lowercase-`json` fence is the closed
`nomifun.creative-studio.canvas-ops/v1` artifact. A lexical pass rejects duplicate
decoded JSON keys before ordinary parsing can silently choose the last value;
both frontend and backend reject artifact JSON above 256 KiB before materializing
its value graph.
The first allowlist contains add-text, update-text, move, resize, connect and
disconnect only; media nodes, delete, client-owned IDs, config runtime fields,
unknown keys and loose/non-final JSON never receive an apply button. Invalid
target artifacts render a disabled error card rather than falling back to prose
extraction.

“应用到画布” is a deliberate user action. The product first flushes the Editor,
enters a visible product-wide mutation lock, reads its current authoritative
revision, then posts the assistant message ID with the exact ordered operations.
The server independently reloads that owner-bound, completed assistant message,
re-parses its unique final artifact with the same duplicate-key and operation
allowlist rules, and requires its canonical operations to equal the HTTP body.
The transaction rechecks the raw persisted message content to fence a concurrent
edit. It then fingerprints the canonical operation array and commits the Canvas
CAS plus a result receipt in one SQLite transaction. A later deliberate replay
of the same assistant message returns the first minted node/connection IDs and
its original applied revision without changing the graph again; reusing that
message ID for a different payload is a conflict. Gateway tool calls retain
their separate revision-CAS contract and do not impersonate this user-approved
proposal identity.

The HTTP port verifies Canvas identity, result-to-op correspondence, bounded
i64 revisions, and either a first `revision = expected + 1` commit or an explicit
receipt replay whose current Canvas revision has not regressed. It never
automatically retries a response-loss mutation. Both success and failure paths
attempt a guarded authoritative reload without allowing a reload failure to hide
the original outcome. If that read is unavailable, the product remains locked
behind an explicit “重新载入远端” action instead of enabling a stale Editor.
Session recovery returns the receipt-backed assistant IDs, so a remount projects
committed cards directly as “已应用”. While the request and reload settle, Editor
commands, panel mutations, new generation/recovery work and a second proposal
are excluded; route exit waits for the same operation.

The source geometry is canonical for views that currently have no resize handle:
the left library is 280px and opening Agent normalizes the right panel to 390px.
The Agent supplies its own single header; the generic right-panel tab header is
shown only for properties so the product never renders stacked title bars.

The Editor also exposes the canonical pending-task recovery feed described in
`../editor/WIRING.md`. Canvas-scoped image, video, and audio task runtimes are
mounted only after the Canvas and Editor graph are both hydrated. The image
runtime routes two strict persisted operations without creating a second
document controller:

- `image-mask-edit` uploads the blue-marked reference as a hidden real asset.
- `image-node-compose` submits exact `image_generation` / `t2i` only when the
  active image node has neither a base asset nor directly connected image or
  panorama assets. Otherwise it submits exact `image_edit` / `i2i`: the active
  node's base image is pinned first and valid direct media inputs follow durable
  connection order. The inline composer shows that same ordered reference list.
  `@` selections persist occurrence-level node bindings and compile only those
  tokens to `Reference N`; all other authored text remains unchanged. The
  config keeps the authored prompt while task parameters and ordered inputs
  freeze the exact Provider prompt and asset snapshot used for retry/recovery.
  Protocol-specific input policies block unknown or exceeded multi-image
  requests, and adapters reject rather than truncate unsupported extra images.
  A separate product ceiling limits image-edit inputs to eight and the creation
  engine caps decoded input bytes at 256 MiB, so an open-ended Provider
  transport cannot turn a large Canvas fan-in into unbounded memory use.

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

## Compatibility only

The migration reader for `nomifun.creative-studio/v1` may still carry an
internal `projectId`, and legacy project-document/repository adapters may
translate that historical shape. Those adapters are not the public Canvas API:
the product library is `/workshop/canvases`, the route parameter is `canvasId`,
and the Agent endpoint is scoped by `canvasId`.
