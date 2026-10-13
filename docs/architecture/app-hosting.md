# Application hosting architecture

The **Apps** page manages ordinary local programs. The single owner is
[`nomifun-app-hosting`](../../crates/backend/nomifun-app-hosting/src/lib.rs);
HTTP and Agent management tools call that same owner. Applications can provide
a web interface or run in the background without an SDK, Manifest, activation
callback, or frontend Bridge.

The home page provides Create, Import, search, tag filters, a card grid and
pagination over real registered metadata. Application workspaces reuse the full
ordinary conversation on the left and provide metadata, run settings, operations,
output and preview on the right. Manual import/configuration needs no Agent or model.

The workspace fixes the official `app.developer` Agent; model and reasoning choices
use ordinary Session contracts. Creation makes a current Source record whose owned
directory is its canonical Session workspace. `development_session_id` is a verified
navigation reference, not tool authority or a second binding ledger. Canonical opening
`app_project_id` supplies application lifecycle ownership. Reopening verifies the owner,
application identity, ordinary Conversation purpose, fixed template
and exact workspace. Existing Sessions are reused; missing ones are opened through
the existing owner without copying transcripts or restoring retired data.

The immutable built-in App role includes maintaining the application's actual
metadata, run configuration and preview as part of ordinary development. Detailed
connection guidance remains on demand. An explicit new submission can refresh a
current-generation blank App role through the existing idle canonical binding
transition, preserving model, grants, resources, history and immutable artifacts.
Reads, polling and reopening do not silently replace a Snapshot.

The inspector follows the same owner after Agent changes. A real running preview
loads inline, and a new managed PID loads the new program while normal polling and
resizing keep the existing page. Manual drafts retain only edited fields; sparse
saves merge with current configuration inside the application gate.
A completion update may include a non-persisted `completion_observation` for its
exact Session/Turn. A successful file change reloads an already loaded page once
if its PID/URL still matches the observed turn-start baseline. A new PID, read-only
completion, duplicate notification or dismissed preview causes no additional load.
Stopping hides the page and collapses enlargement while retaining prior preview
intent for a production restart. Explicit dismissal stays closed. Transient owner
read failure preserves the current workspace, conversation, drafts and preview,
showing a retryable error; cached display is never Runtime context or admission authority.

Preview embeds the application's ordinary HTTP(S) page on request and supports
enlargement and browser opening. Renderer-origin pages are rejected. The sandboxed
iframe uses `no-referrer` and receives no LocalTrust, SDK or platform Bridge. Remote
WebUI does not open Host loopback URLs and has no cross-machine proxy. This is an
ordinary preview container, not a revived SDK Surface or descriptor protocol.

Folder and ZIP imports create a managed copy under
`{data_dir}/app-hosting/apps/{app_id}` and preserve the original source. Current
settings and exact process observations are written atomically to
`registry/{app_id}.json`. Project and static output paths stay inside this copy.
ZIP path traversal and links are rejected; folder imports reject external and
directory links and materialize internal file links.
Records contain name, description, tags, source/artifact type, ordinary settings
and a nonexclusive development Session reference. Workspace import copies external
input into that same record and detects commands again, preserving ID, Session and
business files absent from the source. Ordinary overwrite preserves current settings.

Existing updates accept non-persistent `detect_settings: true` to save supported
defaults from current files without starting a process. Omitted fields preserve
current values and explicit null clears optional settings. Completion-driven
requests resolve the exact accepted input into its canonical operation and verify
the latest successful terminal, idle state and unsettled effects; notification
message UUIDs are not operation IDs. Only successful file effects on the exact
current bound workspace trigger configuration detection; read-only completion
preserves the record and historical resource definitions do not confer current
workspace authority. The exact completion channel may wait up to two seconds for
the same admission write fence because notification can precede the admitted
turn's read-fence release. All canonical guards still run after acquisition;
manual file changes keep immediate concurrency rejection and no effects replay.
GET and history reopening have no detection
side effects. Empty preparation steps and preview's actual start-command fallback
are shown explicitly in the form.
Platform copy, write, start and delete operations check every owned path
component from the hosting root, including the root itself. Windows junctions
and other reparse points do not become a new ownership anchor. Replaced paths
are rejected while registration and external data remain; stop retains its
existing exact process authority.

Each application serializes its own settings, preparation, start, stop and
deletion. Slow commands do not hold the global application map. First preview
runs install then preview, or install, build and start if preview is absent.
First use runs install, build and start; daily start runs only start. Rebuild
stops the old tree before install, build and start. Empty optional steps are
skipped and failures stop the sequence. Preview and use share current files and
data. Static files use an ordinary Node HTTP service preset; server artifacts
use their actual foreground command.

The shared [`nomi-process-runtime`](../../crates/shared/nomi-process-runtime/src/lib.rs)
owns each foreground process and its tree. MCP never launches a second process.
Configured local HTTP(S) addresses receive a bounded two-second TCP listening
wait before startup returns. No HTTP, TLS or credentials are sent and remote
domains are not probed. The existing cancellation and real-exit checks remain;
timeout preserves process-state semantics without certifying application health.
Unexpected exits, including exit code 0, are failures. Normal stop and preview
switches are not. Output is bounded to 32 KiB and redacted before exposure or
persistence, with streams kept separate until redaction. Host shutdown stops
managed services. On restart, recovery verifies PID, native creation key and
available executable identity before cleanup; it never trusts a PID alone or
automatically relaunches the application.

System Node is preferred; custom executable paths are tested without a version
allow-list. Node, npm and Bun are detected separately. Missing tools expose
their real error and the official Node download link without replacing an
existing environment.

Desktop and Headless reuse per-start LocalTrust and the installation owner.
Only preview/start receive `NOMIFUN_HOST_URL`, `NOMIFUN_LOCAL_TRUST` and optional
`NOMIFUN_AGENT_BINDING_ID`; install/build actively remove them. Credentials stay
in backend memory and out of settings, browser assets and logs. Web applications
call the existing REST API through their own backend. External authentication,
administrator setup, installation tokens and CORS remain intact.

NomiFun application development (`app.hosting`) is a system capability candidate.
Only the built-in `app.developer` template enables all ten actions by default;
other official templates leave it unselected. Explicit module and
action choices enter the existing frozen Revision/Snapshot; prior personal
choices and Sessions gain no actions automatically. `read_app_guide` loads the
connection guide only when called. Actions are guide/current/import/list/update/
preview/rebuild/start/status/stop. Each turn preactivates only the final granted,
active hosting actions using their frozen schema and existing execution checks,
so a truncated tool search cannot omit current application management actions.
Agents without the capability gain no application context. `current_app` resolves only the exact canonical
workspace; `update_app` atomically saves metadata and commands without authoring
connection bindings or Session ownership. Workspace imports specify the current
application ID rather than registering another copy. Applications use the Agent's
existing coding tools, with no new authoring pipeline, checkpoint or global hook.
Import/start/stop use the existing HostedEffectReceipts and canonical effects
under exact Session, Turn, operation, Snapshot and committed Tool causation.
The Host retains mutation and settlement together. Known partial failures are
recorded as failures; an unknown worker result remains unsettled and cannot
authorize automatic replay. Ordinary apps with no MCP endpoint do not publish
the tool catalog, and identical catalog probes preserve the current generation.
Real catalog changes retain the existing frozen contract checks.

Optional MCP endpoints are read directly from application settings by the
existing MCP owner. Agent/session choices are explicit; application tools are
excluded from global defaults. Connection failures stay local. Tool capacity,
contracts, frozen resources and canonical binding transitions remain governed
by the current Agent chain. Applications use the existing REST
`open / turn / observe / cancel` operations rather than a second Agent service.
Agent changes require a stopped application with no unconfirmed process and
reuse the existing RemoteBinding CAS on the same ID. Settings failures are
compensated through the original owner before releasing the application gate;
unconfirmed cleanup retains the failed registration and connection reference.
Host shutdown cancels preparation and stops managed trees before awaiting HTTP
drain, so an install request cannot block its own cleanup.

## Application projects and owned Sessions

Each application corresponds to one independent project whose root is the managed
directory owned by that application ID. Different applications do not share project
roots, even when imported from the same external source: each gets its own copy.
A command's relative subdirectory does not change that project identity.
Projects reuse the existing canonical Session, Workspace and Runtime without a
project database or copied chat log. The application's internal creation command
records its exact ID in `session/opening.metadata.app_project_id`. The ordinary
creation API cannot supply application ownership or use the application's reserved
creation namespace. `GET /api/apps/{id}/development-sessions` pages by exact owner
and canonical application identity, including archived Sessions. Agent/model changes
and directory relocation retain that identity; new forks inherit it. An ordinary
Session using the same directory, Agent or title remains ordinary and is outside
application history and cascade deletion. Existing current-generation application
Sessions are recognized only from their complete exact canonical opening command
identity and original typed workspace, without rewriting facts, using a navigation
reference as ownership, or importing retired data.
`development_session_id` records the most recently opened
session. The editor keeps its selected Session in the URL, so polling or another
window changing that reference does not switch the mounted conversation. The App
editor only admits Host-verified `app.developer` bindings; application-owned Sessions
switched to another Agent remain readable inside the application without silently
changing their frozen configuration. Cards and the application's **Project sessions**
are the management entry. The ordinary sidebar, ordinary project choices and batch
operations exclude owned Sessions; search and old deep links open their owning App.
If application details are unavailable, an explicit valid Session link can still
show canonical history read-only after verifying the exact Session and App ID.
It uses the existing read-only viewer, does not require the directory to exist,
and creates no registration or archive entry. Network failure does not imply deletion.

New sessions reuse ordinary Session opening and explicit model selection. A stable
creation key is reused after an unknown result. Exact canonical opening recovers an
already created Session even after the application is renamed; conflicting request
parameters are rejected. Explicit full Session deletion removes opening and content
through the existing privacy contract, with no separate creation-key ledger retained.
The application owns the lifecycle of source, commands, preview, settings and its
dedicated Sessions. Chat content stays independent in each canonical Session.
Switching only remounts the conversation area. Completion sync
checks the exact project Session and completed Turn rather than the latest navigation
reference.

New ordinary Sessions with an explicit directory first reuse the current scoped
project-memory resource identity for the same owner and canonical root; only the
first namespace derives its identity from their digest. The same project shares the
existing `project.memory` state, while different projects are isolated. Agent switches
and formal storage relocation preserve that logical identity. The current memory owner remains in-memory for the Host
lifetime; this change does not introduce cross-restart memory persistence. Source
files and canonical Session history retain their existing durable storage.
Existing frozen default memory bindings are unchanged. Runtime
reads only each Session's valid canonical content and authorized memory; it does not
concatenate other sessions' message projections. Ordinary conversations retain
their workspace grouping contract. The App entry does not register an ordinary
display-path preference; confirmed former App preferences are removed.

Relocating the same current dataset validates registered application UUIDs, the real
installation owner and exact owned roots offline. It rebases only copied registry
directories and updates project location parameters through the existing canonical
transition, retaining source, settings, logical resource IDs, grants and original
history under their contracts. Unconfirmed application processes block relocation;
offline inspection never tries to terminate them. See [dataset relocation](agent-session.md#relocating-the-same-current-dataset).

Portable backup/restore and factory reset do not yet fully include the `app-hosting`
root. They therefore do not guarantee complete restoration or removal of application
registrations, project files and Session references. Storage-relocation validation
does not cover those separate workflows.

Manual imports, overwrites and deletion hold the directory fence and existing
admission fences for every current project Session, then recheck the development
reference under the application gate. The directory fence uses the existing in-memory
lock registry and is not persisted. Ordinary creation, fork, admission, warmup and
binding transitions first take that directory's read fence, preventing a new active
Session between the membership query and file mutation. Active Turns,
pending/unknown effects and a live native producer reject file changes.
Sessions in `deleting` are excluded from ordinary history but continue to protect
their exact owner's current explicitly selected project until normal deletion
cleanup finishes. Empty Runtime caches after a restart do not replace that check.
Agent import takes the same directory and project-Session fences within its already
admitted Turn, validating the exact Snapshot, tool causation and this invocation's
pending Effect receipt. Only that invocation's own pending effect is allowed;
other activity and pending/unknown effects still block it. Import also reuses the
existing native workspace write lock so other file tools in the same Turn cannot
race the copy. A denial settles as a pre-execution rejection through the original
Effect contract. Import calls the same application owner without reentering the
manual HTTP fence. Reads, metadata/settings saves and application stop remain available.

The confirmation names source, local data and owned Session history and displays
the owned Session count. The history `total` counts live Sessions; confirmation uses
`owned_session_count` to include archived and still-deleting owned Sessions.
Generic Session deletion rejects application-owned Sessions and directs management
to the application; ordinary deletion keeps its contract.
Under directory and Session fences, application deletion rejects active Turns,
unknown effects and unconfirmed cleanup, stops the managed tree, purges only owned
Sessions through the existing canonical deletion flow, then removes owned files
and registration. Canonical application identity supplies this scope; the latest
navigation reference never grants whole-Session deletion authority.
An owned Session already in `deleting` resumes its original receipt and cleanup.
The flow avoids acquiring the same Session write lock twice and does not inherit
the ordinary file-change rejection of every deleting Session. Partial failures
retain registration and project files for retry without another deletion ledger.
Original sources, external data, ordinary Sessions and other applications remain.
Ordinary Sessions bound to the same directory are neither terminated nor purged;
their active resources, retained invocations or unresolved Git outcomes can still
block physical directory removal. Shared Host credentials remain.

The unused `exclusive_session_ids` field and whole-Session deletion branch are
retired. Only the exact empty array in an existing current registry can be read
and removed on persistence; the public settings contract rejects it. Nonempty or
invalid values fail closed with files, registration and all canonical history
preserved. No Session lookup, conversion or deletion follows those old references.

Retired plugin crates, SDK, Surface/Hook paths and special authoring/delivery
flows no longer serve the product. Published migrations keep their checksums;
forward migration preserves old works, including committed SQLite WAL data,
before dropping retired active tables. Immutable Agent revisions, snapshots and
valid canonical history remain intact; PluginAuthoring is a typed read-only
historical marker. See [Agent Session architecture](agent-session.md).

Run `cargo test -p nomifun-app-hosting --offline` for real command, HTTP, tree,
crash, overwrite and deletion checks.
Use `cargo test -p nomifun-app --test app_workspace_api --offline` for current
metadata, model-free imports, ordinary Session references and file mutation fences.
Also run affected desktop UI, canonical Session and `check:app-hosting-boundary`
checks. The management renderer targets
Tauri and desktop WebUI at a minimum 880×600.

Chinese: [app-hosting.zh.md](app-hosting.zh.md). Usage: [application hosting](../guides/app-hosting.md).
