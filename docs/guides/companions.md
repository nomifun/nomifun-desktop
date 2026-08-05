# Companions

Nomi's virtual companion has grown from "a single nomi" into a **multi-companion
family**: you can create several companions, use them side by side, raise
them separately, and give each its own name, character, persona, and
chat model. Each companion can also be bound to its own **dedicated knowledge
bases** (turning it into a finance companion, a literature companion, a coding companion,
…), and **every memory belongs to exactly one companion** — what one companion
learns is its own, and no other companion can see it. Collection and learning
still run as a single install-wide pipeline, but every memory it produces is
filed under one owner. Memories, companions, and knowledge bases can each be
packed into a `.zip` bundle for export/import, making machine-to-machine
migration painless.

> The entry point is the **Desktop Companion** page in the sidebar (the `/nomi`
> route); the right-click menu of any desktop companion window ("Open chat")
> deep-links there too.

## Page layout: companion switcher + two tab domains

The top of the Desktop Companion page is the **companion switcher bar**: one card per companion
(character thumbnail + name + level) plus a **New companion** button. The
selected companion drives the **companion-domain** tabs below; the few settings
that are still one-per-install live in the **install-wide** tabs:

| Domain | Tab | Contents |
| --- | --- | --- |
| Companion domain (follows the switcher) | Overview | **Desktop-companion toggle** + that companion's level / XP / mood |
| | Memory | **That companion's own memories** — no other companion's rows are listed here, and none of them can read these |
| | Chat | That companion's own companion threads |
| | Model & Knowledge | Chat model picker / **knowledge bindings** |
| | Remote | That companion's IM bots (bound per companion — see the [channels guide](./channels.md)) |
| | Settings | Name / character / persona / quiet hours / delete companion |
| Install-wide (one per install) | Collect · Learn | The single collection + learning pipeline: one set of event sources, one schedule, one learn model for the whole machine. Editing it from any companion changes it for all of them |
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
4. **Deleting a companion** cascades: its **memories**, skills, companion
   conversations, runtime state (XP, …), and `('companion', companionId)` knowledge
   bindings are removed together; if you delete the default companion, the
   default role moves on to the next one. Since every memory belongs to one
   companion, deleting a companion permanently destroys the memories filed under
   it, and no bundle can bring them back (the companion bundle carries settings
   and growth only, and there is no memory-export UI per companion). Deleting
   down to zero companions is allowed — collection and learning keep running,
   but with nobody to own new memories a learning run stops early instead of
   writing them.

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

## Memory ownership, collection, and learning

**Every memory belongs to exactly one companion.** A companion's prompts are
injected with its own memories and nothing else, its Memory tab lists its own
rows only, and there is no way — in the UI or the API — to move a memory from one
companion to another. What A learned, only A knows.

The *facilities* under `{data_dir}/companion/shared/` are still one per install
(one config, one event tree, one `memory.db` file holding every companion's rows
side by side, each row tagged with its owner) — but the pipeline's **output** is
always filed under a single owner:

- **Collection** — a single pipeline subscribes to the global event
  bus, gathers your working data according to the collect switches,
  and writes `shared/events/YYYYMMDD.jsonl`. Raw events default to a
  **30-day target retention and a hard 64 MiB capacity**. Expired files are
  removed after every enabled learning/evolution consumer has processed them;
  the capacity boundary always wins and may evict the oldest unprocessed day.
  Data Sources lets you configure 7–365 days and 16–512 MiB and shows current
  usage plus the stored date range. There is no raw-row viewer or manual-clear
  action. A memory bundle containing raw events must fit the current hard cap
  before anything is imported; after a successful import, the same retention
  cleanup runs immediately.
- **Learning** — a single learner incrementally distills events into
  long-term memories on the configured interval, stored in
  `shared/memory.db`. The learning pipeline uses the **learn model
  from the shared config** (independent of each companion's chat model — one
  pipeline, one budget). Its schedule, model, and event sources are
  **install-wide**: there is one loop for the whole machine, and editing it from
  any companion's tab changes it for all of them.
- **Who owns what the learner writes** — the memories a learning run produces are
  filed under the **owner companion**: the explicit default companion if it is
  still in the roster, otherwise the oldest companion. Only that companion will
  ever see them. Memories saved during a chat instead belong to the companion in
  that conversation.

### XP and mood attribution

| Source | Credited to |
| --- | --- |
| Learning-run output (scored by events processed + new memories) | **All companions** (the family grows together) |
| Companion chat turn (+2) | Only the companion in that conversation |
| Memory saved during chat (+5) | Only that companion |

**Mood is global**: it is produced by learning runs and stored in
shared state, so all companions share one mood (per-companion mood/personality
divergence is reserved for a later version). XP and mood are the only things
still pooled across the family — memory is not.

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
- The companion can also **grow its own libraries**: the platform Gateway
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
"literature companion", a "coding companion" — persona, model, knowledge, and
memory are all per-companion.

## Binding a companion to a channel

Each IM platform (Telegram / Lark / DingTalk / WeChat) can bind a greeter
companion for remote messages: open the companion's **Remote** tab
(`/nomi?companion=<id>&tab=remote`) and connect or rebind the bot there. A
channel row can hold the direct binding; an unbound row falls back to the
platform preference `channels.{platform}.companionId`. If neither resolves to
a live companion, the channel remains unbound rather than acquiring an
implicit identity. Switching or deleting a binding resets the affected active
sessions so the next message resolves ownership again. See the "Channel Agent
integration" section of the [Channels guide](./channels.md).

> A companionId grants no permissions: it selects persona / model / knowledge
> mounts, and it decides which companion's memories the conversation reads and
> writes. Platform Gateway availability is
> derived server-side from the authenticated instance-owner boundary; it is
> never granted by companion or Conversation metadata.

## Export / import: migrating between machines

The install-wide **Migrate** tab offers three kinds of `.zip` bundles
(the migration UI is desktop-only; paths are picked with the system
dialog):

| Bundle | Contents | Import semantics |
| --- | --- | --- |
| **Memory bundle** | Every companion's long-term memories in one file + mood; **optionally** the raw event data (checkbox) | **Merged with dedup** into local memories (original timestamps and sources preserved). Ownership is **re-homed on import**: companion ids are not stable across machines, so every imported memory lands on the owner companion (explicit default if present, else the oldest) unless its original owner id happens to exist locally. Dedup compares within one owner only |
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

> A memory bundle does not preserve the split between companions: the whole
> pile arrives on one companion. Pick which one by making it the default
> companion **before** importing. If the roster is still empty, the memories are
> parked and land on the first companion you create (at a later launch).

### Privacy boundaries

- `events/*.jsonl` is **raw collected data containing your working
  content verbatim** — it is **not** exported by default; it only
  enters the memory bundle when you explicitly tick "include raw event
  data".
- **Neither memories nor chat history travel with the companion bundle**:
  memories live in `shared/memory.db` and companion conversation logs live in
  the main database, while the companion bundle carries only persona, settings,
  and growth. Memories move only through the memory bundle (which is
  install-wide, not per companion); chat logs stay on the original machine.

## Automatic migration of legacy data

After upgrading from the single-companion version, the first boot detects the
legacy layout `{data_dir}/companion/nomi/`: if it exists and `companion/shared/`
does not, it is automatically migrated into the install-wide `shared/` files plus
a first companion (default name **"Nomi"**, inheriting the existing XP /
persona / character / model / desktop-companion position / companion
threads). The migration is idempotent and re-entrant; on completion a
`.migrated` marker is written into the legacy directory, which is kept
around (to be cleaned up after one release cycle). No manual action is
needed.

### Upgrading from shared memory: memories get one owner

Memory used to be shared by the whole family. On the first launch after this
upgrade, every previously shared memory is **re-homed onto a single companion**:
the explicit default companion if it is still in the roster, otherwise the oldest
one. Nothing is deleted and nothing is duplicated — the rows keep their ids,
content, timestamps, strength, and pinned/archived state.

**But every other companion loses sight of them, and this cannot be undone.** A
memory has no "share again" switch, and re-homing it to a different companion is
not possible in the UI or the API. If you want a specific companion to inherit
the family's history, make it the default companion **before** you launch the
upgraded build. Companions created afterwards start with no memories at all.

If the roster is empty at that launch, the rows are simply left as they are:
they stay readable by every companion (there is none), and the re-homing runs at
the first launch that has one.

## Manual walkthrough checklist

To verify a multi-companion setup end to end, walk through in order:

1. **Create two companions**: create companions A and B, rename them, change
   characters; confirm the first one carries the "default" badge.
2. **Bind one base each**: bind knowledge base X to A and Y to B (companion
   Model & Knowledge tab → Knowledge).
3. **Retrieval isolation**: in A's and B's chats, ask about content
   that only exists in X / Y respectively; confirm A only hits X and B
   only hits Y.
4. **Memory isolation**: in A's chat, have it remember
   something (save a memory); switch to B's chat and ask about it — B must
   **not** know it, and the memory must appear only in A's Memory tab.
5. **Export/import roundtrip**: export the memory bundle + A's companion
   bundle + base X's bundle; (on a new machine or after a wipe) import
   in the order knowledge base → companion → memory; confirm the rebuilt A
   has its binding restored automatically and memories merge without
   duplicates — all of them under the local owner companion.
6. **Channel companion switch**: on some channel platform, switch the greeter
   companion from A to B; confirm the active sessions are reset and the next
   remote message is greeted with B's persona and B's knowledge mounts.

## Routes & API

| What | Where |
| --- | --- |
| List / create companions | `GET/POST /api/companion/companions` |
| Companion detail / update / delete | `GET/PATCH/DELETE /api/companion/companions/{companionId}` |
| Install-wide config (collect / learn / default companion) | `GET/PATCH /api/companion/config` |
| One companion's memories / add a memory | `GET /api/companion/memories?scope_companion_id={companionId}`, `POST /api/companion/memories` (`scope_companion_id` = the owner; omitted lets the server resolve it) |
| Edit a memory (content / pin / status only — never its owner) | `PUT /api/companion/memories/{memoryId}` |
| Per-companion companion threads | `GET /api/companion/companions/{companionId}/companion/threads`, `…/companion/active` |
| Export memory bundle | `POST /api/companion/export/memory` (`{dest_path, include_events}`) |
| Export companion bundle | `POST /api/companion/export/companions/{companionId}` |
| Import memory / companion bundle | `POST /api/companion/import` (dispatched by manifest.kind) |
| Export / import knowledge-base bundle | `POST /api/knowledge/bases/{id}/export`, `POST /api/knowledge/bases/import` |
| Bind a companion to a channel | `POST /api/channel/settings/companion` |

## Related

- [Channels](./channels.md) — Channel Agent integration and per-platform
  companion binding.
- [Data and Storage](../architecture/data-and-storage.md) — the `companion/`
  data directory layout.
