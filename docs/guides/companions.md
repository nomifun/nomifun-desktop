# Companions

Nomi's virtual companion has grown from "a single nomi" into a **multi-companion
family**: you can create several companions, use them side by side, raise
them separately, and give each its own name, character, persona, and
chat model. Each companion can also be bound to its own **dedicated knowledge
bases** (turning it into a finance companion, a literature companion, a coding companion,
…), while every companion **shares one memory hub** — collection and learning
run as a single global pipeline, so whatever one companion learns, the whole
family remembers. Memories, companions, and knowledge bases can each be
packed into a `.zip` bundle for export/import, making machine-to-machine
migration painless.

> The entry point is the **Desktop Companion** page in the sidebar (the `/nomi`
> route); the right-click menu of any desktop companion window ("Open chat")
> deep-links there too.

## Page layout: companion switcher + two tab domains

The top of the Desktop Companion page is the **companion switcher bar**: one card per companion
(character thumbnail + name + level) plus a **New companion** button. The
selected companion drives the **companion-domain** tabs below; everything that is
global lives in the **shared-domain** tabs:

| Domain | Tab | Contents |
| --- | --- | --- |
| Companion domain (follows the switcher) | Overview | **Desktop-companion toggle** + that companion's level / XP / mood + shared stats |
| | Chat | That companion's own companion threads |
| | Model & Knowledge | Chat model picker / **knowledge bindings** |
| | Remote | That companion's IM bots (bound per companion — see the [channels guide](./channels.md)) |
| | Settings | Name / character / persona / quiet hours / delete companion |
| Shared domain (one per install) | Memories · Collect · Learn · Suggestions | The shared memory hub (one copy for all companions) |
| | Migrate | Export / import migration bundles (see below) |

## Creating and managing companions

1. Click **New companion** on the switcher bar, pick a name and one of the
   six characters (mochi / ink / roux / pixel / bolt / boo).
2. **The first companion automatically becomes the default companion** (its card
   carries a "default" badge). The default companion is the fallback whenever
   a channel has no explicit binding (see the channels section below).
3. In a companion's **Settings** tab you can rename it at any time (takes
   effect immediately), swap the character, tune the persona (preset or
   custom), **pick a chat model just for this companion**, and toggle the
   desktop companion plus its quiet hours.
4. **Deleting a companion** cascades: its companion conversations, runtime
   state (XP, …), and `('companion', companionId)` knowledge bindings are removed
   together; if you delete the default companion, the default role moves on
   to the next one. Deleting down to zero companions is allowed (the shared
   memory hub exists independently of companions — collection and learning
   keep running).

On disk each companion is a directory — `{data_dir}/companion/companions/{companion_id}/config.json`,
**the directory is the source of truth** — which is also the unit the
companion bundle exports and imports.

### Multiple desktop companions on screen

Every companion with the desktop-companion switch enabled gets its own desktop
window (transparent, always-on-top, draggable; window label
`companion-{companionId}`). Several can share the screen; keeping it to 5 or fewer
is recommended (each window is an independent WebView instance — the
UI warns about performance beyond that but does not enforce a limit).
Right-click any desktop companion to jump straight to its chat.

## The shared memory hub

All companions share one set of memory facilities under
`{data_dir}/companion/shared/`:

- **Collection** — a single pipeline subscribes to the global event
  bus, gathers your working data according to the collect switches,
  and writes `shared/events/YYYYMMDD.jsonl`.
- **Learning** — a single learner incrementally distills events into
  long-term memories on the configured interval, stored in
  `shared/memory.db`. The learning pipeline uses the **learn model
  from the shared config** (independent of each companion's chat model — one
  pipeline, one budget).
- Memories saved during any companion's chat, and memories produced by
  learning, are **visible to every companion** — switch companions mid-stream and
  the new one remembers everything that happened before.

### XP and mood attribution

| Source | Credited to |
| --- | --- |
| Learning-run output (scored by events processed + new memories) | **All companions** (the family grows together) |
| Suggestion adopted (+20) | **All companions** |
| Companion chat turn (+2) | Only the companion in that conversation |
| Memory saved during chat (+5) | Only that companion |

**Mood is global**: it is produced by learning runs and stored in
shared state, so all companions share one mood (per-companion mood/personality
divergence is reserved for a later version).

## Binding knowledge bases to a companion

In a companion's **Model & Knowledge tab → Knowledge** section, use the binding
control to mount one or more knowledge bases on that companion (the binding
is `('companion', companionId)`). Scope of effect:

- The companion's **companion chats** and the **channel conversations** it
  greets (conversations carrying `extra.companionId`) mount that companion's bound
  knowledge bases — searchable during the conversation. Regular
  conversations without a companionId keep their conversation-level bindings;
  the two are **not merged**.
- **What the agent sees**: bases are mounted at
  `{workspace}/.nomi/knowledge/`, and the injected context carries, per
  base, the description + an AI digest + "when to consult" hints + a
  budgeted table of contents (20 entries per base / 60 global,
  directories aggregated beyond that), plus an explicit retrieval
  protocol — the agent is told to look things up rather than answer
  from memory.
- **Write-back** comes in two modes, briefly:
  - **staged** — knowledge produced during a conversation first lands
    in the base's `_inbox/` (isolated per conversation) for you to
    review on the knowledge page before it is committed;
  - **direct** — skips staging and writes straight into the base.
- **AI bootstrap**: the **AI generate** button on the knowledge page
  (list edit modal and detail page) calls
  `POST /api/knowledge/bases/{id}/autogen` to produce the base's
  description and `README.md`; a `.zip` import auto-fills an empty
  description. Requires a configured AI provider (`409` otherwise).
- **URL sources**: a base can be created from up to 16 URLs.
  *snapshot* mode fetches them at creation, converts each page to
  markdown under the base's `snapshots/` (pages over 32 KB are
  AI-compressed) and auto-generates the digest — refreshable from the
  detail page; *live* mode lets the agent fetch at runtime (engines
  without a web tool can call the gateway tool
  `nomi_knowledge_fetch_url`). Only public `http/https` URLs are
  accepted (SSRF guard).
- The companion can also **grow its own libraries**: the Desktop Gateway
  ships seven knowledge tools (list / bindings / create / write /
  autogen / fetch-url), and knowledge-deposit tips are built into the
  companion's system prompt — a companion or channel chat can create a base
  and distill notes into it unprompted. When
  `nomi_knowledge_create_base` is called with `urls`, the fetching runs
  as a background job — the tool returns immediately, so the agent must
  not create the base again just because the snapshots haven't appeared
  yet; once the base's description shows up, the fetch + digest
  pipeline has finished.

Bind different bases to different companions and you get a "finance companion", a
"literature companion", a "coding companion" — persona, model, and knowledge are
all per-companion, while memory stays shared.

## Binding a companion to a channel

Each IM platform (Telegram / Lark / DingTalk / WeChat) can bind its own
greeter companion for remote messages: open the companion's **Remote**
tab (`/nomi?companion=<id>&tab=remote`) and connect or rebind the bot
there. The binding is still persisted as `assistant.{platform}.companionId`
for legacy platform-level preferences when a channel row has no direct
companion binding. With no binding the **default companion** takes over;
switching the binding resets that channel's active sessions (the next
message is greeted by the new companion); if a bound companion is deleted,
the platform falls back to the default companion and the sessions are
likewise reset. See the "Master Agent mode" section of the
[Channels guide](./channels.md).

> A companionId grants no permissions (memory is shared anyway): it only
> selects persona / model / knowledge mounts — unlike the
> `desktopGateway` marker, which grants gateway tools.

## Export / import: migrating between machines

The shared-domain **Migrate** tab offers three kinds of `.zip` bundles
(the migration UI is desktop-only; paths are picked with the system
dialog):

| Bundle | Contents | Import semantics |
| --- | --- | --- |
| **Memory bundle** | All long-term memories + learning history + mood; **optionally** the raw event data (checkbox) | **Merged with dedup** into local memories (original timestamps and sources preserved) |
| **Companion bundle** | One companion's persona / character / settings / XP + the **name list** of its bound knowledge bases (`knowledge_refs`) | Creates a new companion under a fresh id, name conflicts get a "(2)" suffix; knowledge refs are matched **by name** against local bases to rebuild bindings — unmatched names are listed so you can import those knowledge bundles first and bind manually |
| **Knowledge-base bundle** | Base metadata + the md file tree verbatim | Lands as a new knowledge base, name conflicts get "(2)" |

Migration steps:

1. Old machine: export the **memory bundle** (tick events only if you
   want them) → export a **companion bundle** per companion → export a
   **knowledge-base bundle** per base.
2. New machine: import the **knowledge-base bundles** first (so companion
   bundles can rebuild bindings by name) → then the **companion bundles** →
   then the **memory bundle**.
3. Check each companion's model setting: model config travels verbatim in
   the bundle, but if the new machine has no matching provider it shows
   as unconfigured — re-select in settings.

### Privacy boundaries

- `events/*.jsonl` is **raw collected data containing your working
  content verbatim** — it is **not** exported by default; it only
  enters the memory bundle when you explicitly tick "include raw event
  data".
- **Chat history does not travel with the companion bundle**: companion
  conversation logs live in the main database; the companion bundle carries
  only persona and settings. Chat logs stay on the original machine.

## Automatic migration of legacy data

After upgrading from the single-companion version, the first boot detects the
legacy layout `{data_dir}/companion/nomi/`: if it exists and `companion/shared/`
does not, it is automatically migrated into the shared memory hub plus
a first companion (default name **"Nomi"**, inheriting the existing XP /
persona / character / model / desktop-companion position / companion
threads). The migration is idempotent and re-entrant; on completion a
`.migrated` marker is written into the legacy directory, which is kept
around (to be cleaned up after one release cycle). No manual action is
needed.

## Manual walkthrough checklist

To verify a multi-companion setup end to end, walk through in order:

1. **Create two companions**: create companions A and B, rename them, change
   characters; confirm the first one carries the "default" badge.
2. **Bind one base each**: bind knowledge base X to A and Y to B (companion
   Model & Knowledge tab → Knowledge).
3. **Retrieval isolation**: in A's and B's chats, ask about content
   that only exists in X / Y respectively; confirm A only hits X and B
   only hits Y.
4. **Shared memory round-trip**: in A's chat, have it remember
   something (save a memory); switch to B's chat and ask — confirm B
   knows it.
5. **Export/import roundtrip**: export the memory bundle + A's companion
   bundle + base X's bundle; (on a new machine or after a wipe) import
   in the order knowledge base → companion → memory; confirm the rebuilt A
   has its binding restored automatically and memories merge without
   duplicates.
6. **Channel companion switch**: on some channel platform, switch the greeter
   companion from A to B; confirm the active sessions are reset and the next
   remote message is greeted with B's persona and B's knowledge mounts.

## Routes & API

| What | Where |
| --- | --- |
| List / create companions | `GET/POST /api/companion/companions` |
| Companion detail / update / delete | `GET/PATCH/DELETE /api/companion/companions/{companionId}` |
| Shared config (collect / learn / default companion) | `GET/PATCH /api/companion/config` |
| Per-companion companion threads | `GET /api/companion/companions/{companionId}/companion/threads`, `…/companion/active` |
| Export memory bundle | `POST /api/companion/export/memory` (`{dest_path, include_events}`) |
| Export companion bundle | `POST /api/companion/export/companions/{companionId}` |
| Import memory / companion bundle | `POST /api/companion/import` (dispatched by manifest.kind) |
| Export / import knowledge-base bundle | `POST /api/knowledge/bases/{id}/export`, `POST /api/knowledge/bases/import` |
| Bind a companion to a channel | `POST /api/channel/settings/companion` |

## Related

- [Channels](./channels.md) — channel Master Agent mode and per-platform
  companion binding.
- [Data and Storage](../architecture/data-and-storage.md) — the `companion/`
  data directory layout.
