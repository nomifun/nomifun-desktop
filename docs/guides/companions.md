# Companions

Create multiple companions with separate names, figures, personas, models and knowledge bindings. Each companion owns its memories, learning settings, skills, XP and mood. Collection settings apply to the installation. Memories, companions and knowledge bases support ZIP export/import.

Each companion selects one primary Agent, default chat model and Skill/MCP configuration. Desktop, quick-window, approved IM and physical devices are entry points with independent canonical Sessions. Ordinary work runs directly in the receiving Session; long work can run in the background or in parallel when useful. See the [companion architecture and acceptance status](../architecture/companion.zh.md) (Chinese).

Companion replies use natural language that can be read aloud directly. Kaomoji,
emoji, meme or sticker markers, and action or stage directions are prohibited;
emotion is expressed through words. Custom personas, memories and skills cannot
override this rule. Desktop bubbles and robot speech filter recognized
nonverbal decorations to keep conversations and speech coherent.

> The entry point is the **Desktop Companion** page in the sidebar (the `/nomi`
> route); the right-click menu of any desktop companion window ("Open chat")
> deep-links there too.

## Page layout: roster + Cohabit / Manage

The Desktop Companion page owns the whole companion experience. A persistent
roster on the left switches identity without mixing companions into the work
Session tree. The selected companion then has two first-level modes:

| Mode | Surface | Contents |
| --- | --- | --- |
| Cohabit | Multiple conversations | Independent entry-point sessions and histories, plus a concise status / shared-memory / connection summary |
| Manage | Overview | Name, figure, desktop visibility, level, persona, one Agent and default chat model, and voice / vision slots |
| Manage | Memory & knowledge | Only this companion's memories and knowledge bindings |
| Manage | Channels & Robots | IM channels, physical robots and companion-scoped MCP connections |
| Manage | Evolution / Skills | Learning cadence, collection inputs, retention, quiet hours and generated / configured skills |
| Manage | History / Migration & deletion | Day-based chat reading, package transfer and irreversible deletion |

The native desktop quick window is the lightweight path into the same product:
it switches companions, shows the latest turns, accepts a quick reply, and
deep-links to Cohabit or Manage. It never appears in the work Session sidebar.

The Cohabit toolbar provides a session selector, **New conversation** and entry-point labels.
Its companion-info sidebar offers desktop visibility and mood controls, plus the unified Agent/model settings. Desktop visibility controls the native figure window. Mood follows learning automatically until a manual emotion is selected.

The settings distinguish the current Session from future defaults. Explicit Agent/model changes require an idle Session and settled operations, preserving identity and history. Other Sessions keep their configurations. If the Session update succeeds but saving the default fails, the UI distinguishes the results and offers a retry.

The companion-info sidebar can hide when the conversation pane has insufficient room. Use the sidebar control to restore it after enlarging the pane.

The collapsed figure exposes up to three alternative companions directly. If
the target already has a visible desktop window it is brought forward; otherwise
the target is created at the current position and replaces the current figure
only after its window is ready.
In the expanded quick window, full Cohabit and Manage links live under **More**.
Drag any non-interactive frame area to reposition the native window; collapsing
the panel keeps the adjusted small-window position.
The Guid home may still present the visual companion showcase as a discovery
and deep-link surface; selecting a companion opens Cohabit and never creates or
inserts a companion row in the work Session tree.

## Creating and managing companions

1. Click **New companion** on the switcher bar, pick a name and one of the
   two official characters, Mochi (`mochi`) or Bolt (`bolt`), or create/import a custom figure.
2. **The first companion automatically becomes the default companion** (its card
   carries a "default" badge). Channels still require an explicit binding and do not acquire the default companion automatically.
3. In **Manage → Overview** you can rename it, swap its character, tune its
   persona, **pick the companion's default Agent and chat model**, and toggle the desktop window. Quiet hours
   and learning controls live under **Manage → Evolution**.
4. **Deleting a companion** cascades: its **memories**, skills, companion
   conversations, runtime state (XP, …), and `('companion', companionId)` knowledge
   bindings are removed together; if you delete the default companion, the
   default role moves on to the next one. Since every memory belongs to one
   companion, deleting a companion permanently destroys the memories filed under
   it — export that companion first if you want to keep them (its bundle carries
   its memories by default). Deleting
   down to zero companions is allowed — collection keeps running, but with no
   companion left there is no learning loop at all (the schedule belongs to the
   companion), so events simply accumulate until you create one.

Each companion's configuration is saved at
`{data_dir}/companion/companions/{companion_id}/config.json`. A companion package
exports one companion and imports it as a new companion. Canonical Session facts own session affiliation and
history; directory names do not establish ownership.

An existing companion with a retired or unrecognized character ID uses Mochi
while keeping its identity, settings, memories and conversations. Imported
packages use the same appearance fallback and retain the usual cloning rules:
they create a new companion ID and do not import the original conversations.

### Changing a companion's Agent

Use **Manage → Overview → Model configuration** or the Cohabit sidebar to select the primary Agent. The default is the official **Companion** (`companion.default`); personal Agents must provide the companion identity and memory core. Physical devices use the same setup with independent Sessions.

The workbench exposes the complete capability and Action catalog. Chat, research, knowledge, files, Office and computer operations run directly with authorized tools. Device operations need explicit Robot capabilities, exact resources and permissions. Unbound or offline devices do not block unrelated work.

After **Edit Agent**, use **Apply latest setup** to update the idle Session explicitly. Saving defaults never replaces other Sessions. Skill/MCP defaults follow the same boundary; current Session choices use the canonical capability picker. ASR, TTS, vision and learning models retain their individual purposes.

Finish or end the current work before changing an Agent while a reply is running,
a Turn is paused for recovery, or effects remain unsettled. If the current
Session was updated but its future default could not be saved, the UI states
that partial outcome and offers **Apply latest setup** to retry; the current
Session continues with its committed setup. Other failed saves do not make the
displayed choice authoritative. Switching Agents never transfers
the companion's memories, knowledge bases or robot permissions to another companion.

### Multiple sessions and input routing

**New conversation** creates an independent Session with the same
persona and long-term memory. It has its own working context, without copying
all history or clearing another session. The session selector also lets you
inspect the companion's device sessions, identified by their entry-point label.

Viewing or creating a session does not redirect quick-window input. Select
**Continue in desktop window** to make the viewed desktop receive Session its future
input target; the **Active in desktop window** marker identifies that target.
Robot input resolves by companion and exact device, and IM input follows its
channel binding. Navigating in this page or another window does not redirect
those inputs.

Deleting one session cleans up that Session while preserving the companion's
shared working directory and long-term memory. Deleting the companion invokes
cleanup for all its sessions, effects and directory; active work or unconfirmed
operations can block deletion. Archiving and clearing context do not create a
new session. Inspecting another session does not grant control over its tasks;
questions and approvals still require explicit handling for the corresponding
task.

### Stopping delegated work

The delegation card in the chat and its task detail both provide **Stop
collaboration**. This cancels the collaboration and its unfinished subtasks,
including work that is running, waiting for input, paused or being planned.
**Back to main conversation** changes the view while the work continues;
stopping the main reply also does not cancel work already delegated. The task
history and confirmed results remain available after cancellation, and you can
return to the main conversation to submit another request.

If an interruption leaves the execution outcome unconfirmed, the detail shows
a review warning and a stop control. A normal answer or automatic retry cannot
resume that state because it could repeat operations that already happened.
Stopping does not undo browser clicks, file changes or other completed effects;
unconfirmed operations still need review.

### Multiple desktop companions on screen

Every companion with the desktop-companion switch enabled gets its own desktop
window (transparent, always-on-top, draggable; window label
`companion-{companionId}`). Several can share the screen; keeping it to 5 or fewer
is recommended (each window is an independent WebView instance — the
UI warns about performance beyond that but does not enforce a limit).
Right-click any desktop companion to jump straight to its chat.

## Memory, collection and learning

Every memory belongs to one companion. Its entry points and multiple sessions share
long-term memory and can retrieve that companion's experiences; other companions
remain isolated. Updated shared context is read for subsequent model calls and
does not alter requests already sent. Shared experiences help interpret the
current request and do not automatically rerun old tasks. Learning settings,
generated skills, XP and mood also remain separate. Memories saved during chat
belong to the companion participating in it.

- **Collection** records enabled sources of your working data for learning. This raw event spool is separate from persistent conversation history; recording user input for learning does not remove replies from chat history.
- **Retention** defaults to a 30-day target and a hard 64 MiB capacity. Expired data is removed after enabled consumers process it, but the capacity limit can evict the oldest unprocessed data. Data Sources shows usage and date range; configurable limits are 7–365 days and 16–512 MiB.
- **Learning** is configured in each companion's Evolution page, including its interval and learning model. The learning model can differ from its chat model.
- **Skill generation** can distill repeated work into that companion's skills. Review generated content and the conservative/aggressive setting before relying on it.
- **Quiet hours** suspend that companion's learning, evolution and desktop bubbles. Incoming IM messages can still receive replies.

Disabling collection does not delete existing long-term memories or skills. Raw events contain working content and are excluded from exports by default; a raw-event import must fit the current capacity limit.


## Binding knowledge bases to a companion

In the companion's management page, use the knowledge binding controls to select its knowledge bases. Bindings belong to that companion; ordinary conversations keep their own selections. Knowledge supplements long-term memory and can be managed from **Knowledge** (`/knowledge`).

Choose **manual** writeback to require an explicit request before adding knowledge, or **automatic** writeback for durable, reusable facts selected by the Agent. A knowledge base can be imported as a ZIP or created from public HTTP(S) URLs. Snapshot sources save a local copy that can be refreshed; live sources are retrieved when needed. AI descriptions and summaries require a configured model.

The selected Agent still needs the relevant capabilities and resources. A companion's name or a bound library does not grant new tool permissions.

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
| **Companion bundle** | One companion's persona / character / settings / XP + the **name list** of its bound knowledge bases (`knowledge_refs`), plus its own **memories** (`memories.jsonl`, on by default), optionally its **skills** (rows + `SKILL.md` bodies), and its **custom figure image** when it wears one | Creates a new companion under a fresh id, name conflicts get a "(2)" suffix; every carried memory and skill is **re-homed onto that new id with a fresh row id** (a companion bundle is a clone, not a merge), the figure becomes companion-owned, and knowledge refs are matched **by name** against local bases to rebuild bindings — unmatched names are listed so you can import those knowledge bundles first and bind manually |
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

> On a new machine, memories whose original owner is absent are assigned to the local default companion, or the oldest companion. Set the intended default before importing. A companion bundle is the way to carry one companion's memories into its newly created identity.

### Privacy boundaries

- `events/*.jsonl` is **raw collected data containing your working
  content verbatim** — it is **not** exported by default; it only
  enters the memory bundle when you explicitly tick "include raw event
  data".
- **Chat history never travels**: companion conversation logs live in the main
  database and stay on the original machine. Memories and skills DO travel with
  the companion bundle, but only the ones that companion owns, and only when the
  export scope says so (记忆 is ticked by default, 技能 is not — skill bodies are
  executable, so exporting them is an explicit choice). The install-wide memory
  bundle remains the way to move every companion's memories at once.

## Related

- [Companion architecture and acceptance status](../architecture/companion.zh.md)
  (Chinese) — one Agent, multiple sessions, shared context and on-demand background work.
- [Connect a XiaoZhi robot](./xiaozhi-robot.md) — physical entry point, pairing and connection checks.
- [Channels](./channels.md) — Channel Agent integration and per-platform
  companion binding.
- [Data and Storage](../architecture/data-and-storage.md) — the `companion/`
  data directory layout.
