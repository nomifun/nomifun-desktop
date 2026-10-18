# Agent Session logs and content architecture

AgentSession is the sole fact chain for sessions, turns, content, effects and cancellation. User-facing Conversation APIs project the same `AgentSessionId`. Chat and Coding run through one in-process `nomifun.nomi` Runtime.

The [Companion implementation plan and acceptance status](companion.zh.md) covers
concurrent desktop/Robot roles, multiple Sessions and bounded live shared memory.
Pending acceptance must not be presented as verified physical-device behavior.

This is the current development entry point. Rust types, the Session event registry and the canonical schema own executable contracts. Superseded designs and implementation instructions remain only in Git history.

## Facts and derived data

| Data | Responsibility |
| --- | --- |
| Sessions and binding transitions | Identity, immutable Agent Revision and Snapshot, resources and explicit Agent switches |
| Turns | Accepted input, admission, idempotency, lifecycle and terminal receipts |
| Events and payloads | Ordered semantic facts and their content; the source for context and presentation |
| Effects | One ledger; domain owners interpret effect results |
| Message projections | UI queries; never supply missing Runtime records or infer completion |
| Native checkpoints | Recovery bound to exact build, binding and cursor; not another transcript |
| AgentExecution and domains | Their own business facts, referencing the same canonical Session and Turn |

## Runtime context

Application workspaces use an ordinary Conversation with the fixed official
`app.developer` Agent and user-selected model/reasoning policy. The application's
owned directory is the frozen workspace. Its `development_session_id` is a verified
navigation reference, not authority or another binding ledger. Internal application
creation establishes immutable lifecycle ownership in typed
`session/opening.metadata.app_project_id`. Agent/model switches and directory
relocation keep that identity; new forks inherit it. Ordinary creation cannot supply
application ownership or its reserved command identity. Reopening verifies the
canonical owner, application identity, purpose, template and directory.

Each application owns one independent project directory. A project supports multiple
owned Sessions, and different applications do not share project roots. History is
queried by exact owner and canonical application identity, including archive metadata.
Current-generation App-entry Sessions are recognized only from their exact canonical
opening command, producer, owner, causation and original selected application root,
without rewriting openings or importing retired data. Same-directory ordinary Sessions
and previous ordinary forks remain ordinary. Navigation, display preferences and
Message projections cannot supply ownership. Application Sessions are managed in
their owning App and excluded from ordinary sidebar/batch operations; search and
old deep links return to that App. Session content stays independent in
the canonical store; source directories contain no copied transcript. New Sessions
with an explicit directory first reuse an existing scoped project-memory identity
for the same owner and canonical root; the first namespace uses their digest.
They share existing project-memory state for that project, retain the logical identity
across Agent switches and formal storage relocation, and leave prior default bindings intact.
Project memory retains the current in-memory Host persistence contract; it does
not survive a Host restart. Project files and canonical Session history persist.
Installation-level placeholders for default Session-owned workspaces never identify
an application's project directory.

Only that built-in template defaults to all ten `app.hosting` actions. Other
official templates leave NomiFun application development optional. Prior personal
selections and frozen Sessions gain no actions automatically. The guide loads on
demand; mutations use existing HostedEffectReceipts with exact canonical causation.
Manual file imports, overwrites and deletion hold directory and current project
Session admission fences and reject active Turns or pending/unknown effects.
Creation, forks, binding changes, warmup and admission take a directory read fence
in the existing transient lock registry. This prevents a new Session crossing the
membership scan while files change, without another transcript or ledger.
The existing typed deleting-session query also protects exact owner/project roots
until normal cleanup completes; it grants no execution authority and does not put
deleting Sessions back into ordinary history.
An admitted Agent import takes the same project fences and existing native workspace
mutation lock. It verifies its exact Session, Turn, Snapshot, operation, tool causation
and pending Effect receipt; only that invocation's own pending effect is permitted.
Other active work and unsettled effects remain blockers, and rejections settle through
the existing pre-dispatch rejection contract.

Generic deletion rejects application-owned Sessions. Explicit application deletion
uses directory and Session fences and the existing canonical deletion flow to clean
its owned Runtime resources and Session content before removing files and registration.
Owned Sessions already in `deleting` resume their original receipt and cleanup;
partial failure retains the registration and files for retry without another batch
deletion ledger. Active Turns, pending/unknown effects, live native producers and
unconfirmed cleanup remain blockers. Same-directory ordinary Sessions and other
applications remain untouched; their active resources or unresolved outcomes can
still protect the physical directory. Ordinary creation, context, workspace, switches
and deletion retain their contracts.

Official Agents have stable, owner-scoped editable Presets and use the same immutable revision save path as personal Agents. Release seeds initialize unsaved defaults without overwriting saved customizations. Official product launches resolve the owner's saved configuration; model and extension variants continue through the existing Revision/Snapshot path. Historical official identity is validated from host-owned source, owner, and template identity rather than equality with today's seed or mutable configuration.

`engine_history.rs` reads typed canonical events and resolved payloads. It does not read Message projections to determine model roles. Native closed turns replay their structured journal. Missing Runtime records fail closed rather than falling back to chat text.

Tool bodies serve model context, effect verification and recovery. Within the existing Event/Payload chain, the Store factors formally owned tool arguments, results and their compaction copies into Session-scoped, type-specific content-addressed Payloads. Events retain their individual identities, ordering, owner settlement and Runtime observation semantics. Readers validate the exact slots, ownership, types, lengths and digests in the same transaction before restoring typed content. Append drafts cannot supply the reserved storage-reference metadata; missing or damaged references fail closed. No current file is read to reconstruct historical bytes, and no content table, alternate journal or recovery ledger is added. Unique content counts against the existing Payload budget; expanded content and journal accounting retain their original bounds. Exact native build, binding and authority fences remain unchanged.

Turn and Effect indexes retain lifecycle state and the exact `terminal_event_id`, without a second terminal body. Their readers validate the original start/terminal identities, state and index boundaries and resolve canonical content in the same transaction. Payloads require a database body; there is no unimplemented object-directory mode. Forward migrations remove only redundant columns and preserve every payload identity, body byte and event reference when replacing the table. Unreadable object rows reject migration. The published baseline and migration checksums remain unchanged; current Sessions are preserved and retired Agent data is never converted.

An accepted input that fails or is cancelled before Runtime startup remains valid context together with its explicit terminal fact. It does not acquire fabricated Runtime, tool or completion evidence.

Creation prompts, Cron notices and Execution summaries enter context through formal message events. Text before an explicit Agent transition is data only; it does not carry old grants, handles or completion gates. Context-clear events and the fixed accepted-root cursor bound the window.

User context clearing, automatic Companion window-summary resets and per-run Cron resets use one Session-owner
command. It verifies the exact idle context boundary, joins knowledge write-back work,
settles and tears down the existing Runtime, then rechecks native leases, recovery,
effects, execution ownership and the original boundary under the canonical write fence.
The sole result is an idempotent `context/cleared` event; Session identity, immutable
bindings, visible history and long-term memory remain intact. Archived windows carry
their original typed boundary across summary generation, so intervening inputs invalidate
the reset before teardown. Cron resets use the exact run identity and admission fence;
replaying accepted input never clears or stops successor work. UI/API contract 32
requires an explicit user-action key and retires unsupported Runtime command discovery
and side-question interfaces.

Home, active chats, product conversations and Canvas share command parsing, availability,
dispatch and a viewport-aware overlay above the presentation-only composer. Terminal
commands use explicit terminal actions, with clear-view distinct from Session context
clearing. Local actions never become model input, creation requests, steering or queued
Turns. Opening a command catalog cannot create or warm a Session, and read-only
transcripts remain read-only. Skill selection retains the existing frozen Snapshot and
typed input-hint path rather than a second Runtime command catalog.
An unchanged failed command retains its action key for retries; success or an explicit
draft edit starts a new action, without a separate persistent command ledger.

Current-generation history, immutable Snapshots, model changes and explicit Agent transitions remain supported product semantics. They are verified against identity, sequence, binding and content integrity. Executable recovery requires the native checkpoint's exact build and binding. Closed read-only event history may cross an explicitly validated transition without gaining authority.

Raw Responses reasoning returned by a provider uses the existing `ChatProviderReasoning`
block to retain the exact item identity, content and extension fields in model context.
The Broker binds it to the actual producing attempt's route digest; only that same
Responses route can replay it. UI summaries never supply raw content, encryption or
item identities. Actual reasoning text uses the existing reasoning events. Closed
history, context clearing, compaction and native recovery keep their current content
boundaries and exact build/binding fences, without another fact chain or ledger.
Explicit provider `content: null` and nullable status fields remain unchanged.
Validated streamed text can be displayed without filling native content,
summaries or encryption from it. Replay uses the original item rather than
reconstructing a second reasoning state from display-only thinking.

### Model tool rules and guidance

Standard workspace tools are assembled once from the same Snapshot's authorized Actions,
their canonical schemas and `nomifun-agent-contracts/contracts/tool-model-guidance.json`.
Canonical schemas alone own parameter constraints. The new typed guidance contains usage,
descriptions of existing parameter nodes, valid examples and actual host-platform advice;
it cannot declare another set of types, required fields, unions, defaults, limits or grants.
Names use the existing presentation vocabulary. Resource and Action admission remains with
the original ToolPlan compiler and Kernel.

Duplicate identities, invalid parameter paths and missing canonical fields reject assembly.
The Runtime schema factory, old description-extraction overlay and structural subset checker
are retired; tests use canonical Actions and actual assembled definitions. The Runtime receives
the current ToolPlan without a production dependency on domain schema providers. Workspace
roots remain admitted facts, never copied executable paths, environment snapshots or results.

Guidance changes recompute the actual model-schema digest. New source and data participate in
the original native build digest without changing canonical registration identities, Snapshots,
Session content, database schemas or data generations. Active Turns do not hot-swap rules;
native checkpoints retain exact build/binding fences and history is never backfilled.
`start_process` keeps its canonical `wait_ms=0..30000` range; interactive usage recommends 0
followed by explicit polling rather than restoring the old template's non-production zero ceiling.
The early-visible component notice reads the same guidance and is derived only from actual
component fields in the exact authorized ToolPlan, preserving visibility budgets and grants.

### User-input handoff and owned-resource waiting

`request_user_input` declares the exact question and remaining work; the host verifies the owner,
Session and active Turn. A question supplies neither an answer, authority nor completion evidence.
`report_completion` retains the delivery-evidence gate. The host selects a quiescent closed-turn
handoff or live waiting from the original resource owners, never from a model-authored keep-alive
grant. Waiting uses the existing typed Runtime Event/Payload chain, without a wait table, question
ledger, copied task state or additional checkpoint.

With no resource requiring its continued lifetime, the original terminal-input fence, cleanup
proof and canonical commit close the Turn normally with `user_input_request` on `turn/completed`.
Already-admitted replies use steering; later replies start a new Turn in the same Session. Canonical
context retains the question and unfinished requirements. Quiescent checkpoint recovery settles
only that original handoff, without re-asking, replaying effects or restoring live process handles.

An authorization helper that must survive human action parks the same Turn inside the host.
The model stops sampling and `run_turn` does not return a terminal result. Its owner retains the
execution lease, resources, cancellation and original process deadline. Public `waiting` references
the original question ID. The host registers notifications before reading durable steering and
resource revisions, preventing lost replies at the sleep boundary. Real input, original-owner
observations, expiry and cancellation settle the wait. Owner observations are canonical facts,
not fabricated `poll_process` calls or tool evidence. Waiting consumes no model steps, tool-failure
counts or model no-progress windows, and renews neither budgets nor authorization lifetimes.
A user saying authorization finished is not its success receipt; actual timeout remains a command
attempt result while unfinished work can still require attention.

A provider stop ends one response. One stop-resolution decision uses actual plans, current reports
and resource state to select review, live waiting, native suspension or completion. An unfinished
plan alone never proves failure. Exhausted plan, report or end-intent correction retains the work
for human intervention without changing actual command or tool-failure counts. Live resources
stay with their owner; their exit grants no additional correction attempt. Only quiescent resources
can use native suspension. An owner deferred beyond its soft window retains the acknowledged
checkpoint and canonical tail; suspension creates no renewal or invented checkpoint receipt.
Actual faults, failure-stop policy, unknown Effects and patch recovery retain their
original fences and cannot be escaped by a question.

REST, realtime, history, task progress and domain consumers use the same typed public outcome.
Plan steps describe work progress independently of waiting or suspension; a Turn fault cannot
make every pending item a failed task. A parked Turn accepts its exact steering reply while
retaining new-Turn and queue admission fences. IDMM does not treat legitimate waiting as a stalled
model; Cron and Execution distinguish human attention from failure retries or delivered work.
A closed Cron input handoff settles its existing invocation as the non-fault `input_required`
terminal, consuming that occurrence without a retry or permanent reservation. A live resource
wait remains nonterminal. An exact cancellation settles as `cancelled`, clears failure diagnostics and never
enters failure retries. Domain cancellation and direct cancellation of the linked Session converge on the same
canonical cancellation fact. Both AskUser and Automatic policies route a real input handoff through the existing
WaitingInput boundary; an automatic policy's inability to answer for a person does not fail the task. Human replies
still validate the exact active Link, Attempt, original question and source receipt, preserving user admission and
native authorization fences. Automatic policy cannot fabricate human answers or authorization.
The original Cron receipt worker remains attached across waiting and suspension, freezing its remaining
observation budget until the exact Turn settles. Restart reattaches that read-only consumer from the original
reservation and active-runs fence; it creates no invocation, Turn or ledger and never prepares or sends again.
The existing settlement CAS consumes the terminal result once before ordinary scheduling advances.
Restart never treats a durable wait as proof a process survived:
missing owner proof enters recovery/reconciliation without restoring handles or restarting commands.

Waiting replies carry `expected_wait` with the exact source Turn and canonical wait-event `wait_seq`.
The question remains referenced by its original fact rather than mirrored as client authority.
The original steering writer validates the active wait and single reply consumption in one
transaction. Old notices, replaced questions and concurrent windows cannot inject answers into
a successor. Idempotent retries return their original accepted receipt; changing or removing
the guard cannot replay it. Ordinary steering uses the same writer.

### Execution windows and suspension

Model and journal windows renew the same Turn only after an acknowledged quiescent checkpoint.
Cumulative allowances, process deadlines, cancellation and Effect fences remain in force.
When a soft window expires with original processes still running, the Runtime exposes only
their exact stdin, EOF, poll and cancellation controls, authorized file reads needed for input,
and plan updates. It admits no new process or unrelated work and never unlocks report-only review.
Live handles are not checkpointed. Actual terminal and cleanup observations permit a new
checkpoint at the consumed model step; renewal requires its durable acknowledgement. Empty
polls, poll argument changes and instruction reads do not extend stall protection. Real budget
exhaustion, unknown outcomes, cancellation and missing checkpoint acknowledgement keep the
existing fail-closed behavior.

The renderer shares an idle/running/waiting/paused interpretation of the existing canonical projection
without another persistent lifecycle. Suspension retains the original Turn and input fence;
an explicit end-turn action replaces the execution animation and cancels that exact Turn.
Input reopens only after release is confirmed. Public pause notices preserve known execution,
recovery and cleanup codes at the exact Operation and observed cursor. Private prose stays
private, and a reason code never substitutes for cleanup evidence.

### Model derivation and bundled contract evolution

A model change derives a new immutable Revision/Snapshot from the Session's saved artifacts. It changes only the
canonical Chat route and the identities and digests determined by that route. Capability descriptors, contribution
locks, role Providers, Skills, MCP tools, resource authority and all non-model authorial fields stay frozen. Current
installation defaults and authoring catalogs do not participate in model derivation. The strict non-model equality
check remains an invariant and also governs model-only closed history.

Bundled releases may publish new exact schema, artifact and role contribution identities. Before a new local Turn,
model selection or warmup, the same Session owner can prepare a fresh execution contract at an idle boundary.
The compiler starts from the saved authorial configuration and the frozen role contract key and Provider mount,
then proves that action IDs and grants, effect classes, resources, dependencies, runtime requirements and publisher
identity stay unchanged. Display metadata and exact schema/artifact digests may evolve; this is a forward binding
to the newly published contract, not evidence that old inputs are compatible. Managed packages, MCP tools and Skill
content remain exact. There are no per-release, capability-name or digest compatibility exceptions.

The owner serializes preparation with admission, proves teardown of the previous Runtime and settled effects, and
uses the existing canonical `session/agent-binding-changed` transaction. The Session identity, typed resource
definitions and active capability selection are preserved; the old Revision/Snapshot and prior events are immutable.
The transition makes earlier closed history data-only and excludes its executable journal and completion ledger.
It never resumes a checkpoint, replays an old tool call or imports retired Agent data. Active or paused Turns,
Remote and Attempt bindings, including automation Attempts, do not acquire automatic rebinding authority; unknown effects and pending patch
recovery keep their existing fences. Exact-key input redelivery keeps the original admission rather than preparing
a different binding. Repeated preparation of an already-current binding is a no-op.

Session reasoning uses one native `reasoning_effort` field defined by the shared contract. No lossy mirrors or fallback columns are maintained.

### Switching Agents in a conversation

Selecting another Agent applies the change directly, with a local waiting state in the selector. There is no capability/resource review dialog or handoff confirmation. The host still provides preview and version CAS, uses `continue_task` when a verified handoff is available, and otherwise preserves ordinary data context. Messages, composer drafts and Session identity remain intact; the target Agent takes effect on the next Turn. Active or paused Turns, unsettled effects, patch recovery, Remote and Attempt bindings retain their execution fences and produce a concise explanation.

Resources are resolved under the target Agent's grants; unused assets do not expose tools. A usable explicit project may remain as a working directory with no file-operation grants so a later upgrade preserves that choice. The host resolves available defaults under the target grants; removed or temporarily unavailable optional assets do not block other resources. Actions without their optional resource remain unavailable, without invented connections, credentials or authority. When Knowledge remains readable but loses write-back permission, the switch disables its write-back policy and retains reading; explicit policy updates still verify every mounted base. Workspace and process resources use the same root: a usable explicit project is retained, otherwise the existing host materialization prepares this Session's managed workspace. A lower-level Agent does not fail because an unused former workspace is unavailable. The next Turn still passes target-resource and native Runtime admission. Ownership, contract integrity and storage failures cannot be bypassed as missing assets.

Available global Skill and MCP selections carry forward. Removed Skills, disabled MCP servers and servers without published tools are omitted; Package Skills must also be declared by the target Agent. The existing compiler produces an exact target Snapshot and resource definitions, without reusing old MCP authority or changing old Snapshots. A Canvas or Channel asset mounted in an ordinary conversation does not imply a fixed product identity. Actual Canvas business sessions, Companion, Customer and Cron ownership retain their own binding fences.

### Official preset capability and resource completion

Official seeds grant concrete business Actions rather than deriving authority from persona text or Module names:

| Preset | Default capability boundary |
| --- | --- |
| `chat.minimal` | No Modules or Actions; ordinary text chat gains no tool authority. |
| `assistant.general` | Files, processes, version control, artifacts, project memory, research, schedules, browser and computer operations, media, assets, Office, SSH, knowledge and Session management. |
| `coding.codex` | Files, version control, processes, artifacts, project memory, research, collaboration, SSH, knowledge and Session management; retains headless Runtime requirements without browser or desktop-control dependencies. |
| `app.developer` | Complete Coding baseline plus ten application-hosting actions; its independent project uses ordinary Sessions and the same application owner. Other presets keep application hosting optional. |
| `companion.default` | Desktop Companion: persona, shared memory, channels, schedules and everyday files, processes, artifacts, research, browser, computer, media, assets, Office, SSH, collaboration and management. Physical Robot tools are omitted by default and can be explicitly selected. |
| `companion.robot` | Robot defaults: the same Companion identity and memory, independent device Sessions and four explicit Robot Actions; directed desktop delegation supplies real-time queries, research, knowledge, computer work and elaborate motion planning. Source-scoped task controls and result Turns remain available. Users can explicitly add capabilities. |
| `customer-service.default` | Customer notes, handoff, knowledge retrieval, channel replies and sends, files, processes, artifacts, research and schedules; no default global knowledge-registry, cross-Session or terminal management. |
| `creative-studio.default` | Media, Canvas, assets, templates, Office, files, processes, artifacts, project memory, research, browser, collaboration and schedules; retains its three creative Package Skills. |

General and Companion grant only asset read/write in `creative.workshop`; a missing Canvas cannot hide those Actions. Native Office tools edit or preview asset-library documents, sheets and slides. Local-file workflows continue through authorized files/process tools. Global Skills and MCP selections retain their exact compilation path and do not substitute for native Action grants.

Frozen `CapabilityActionDescriptor.required_resource_operations` has explicit optional semantics. `None` or an omitted field retains the complete Module resource gate; sibling Actions cannot imply that an unspecified Action is resource-free. `Some(empty)` explicitly consumes no resource slots, while `Some(map)` requires a binding of each named kind with every declared operation. Model-visible tools and Kernel admission use the same frozen check. Context/Event consumers keep the Module resource check, and Role Providers keep their exact implementation slots. Missing optional targets disable only dependent Actions; filtering never bypasses owner, Snapshot, active-capability or invocation authority.

Ordinary creation and product entry supply deterministic host resources under the target's frozen grants: Workspace, Process Session, project memory, asset library, Scheduler, and applicable Terminal, Browser and Computer resources. Product identity uses the actual Companion, Customer or Canvas target ID; Workspace and Process share one root. Published Browser/Computer Role contracts may be frozen while their optional live environment remains unbound. Knowledge bases, channels, SSH hosts and robots require actual selections, without invented connections or credentials. Owner mismatches, storage I/O or corruption, contract/Provider mismatches, native checkpoint safety and unknown effects retain their fail-closed boundaries.

Saved official customizations are not overwritten by release seeds. The owner-scoped `GET /api/agent-presets/{preset_id}/recommended-capabilities` returns a read-only merged draft and missing Module/Action/Package Skill additions. Persona, instructions, model, Runtime settings, existing extensions and pinned Skill versions remain authored choices. The UI shows additions before saving through the existing SaveRevision `expected_current_revision` CAS and does not replace unsaved edits. Saving leaves existing Sessions unchanged. A Companion's explicit Apply latest command uses the canonical Agent switch at an idle, settled boundary, preparing and committing the current owned configuration under the Session operation and binding-version fences. Direct Session preview/apply additionally requires the exact previewed target Snapshot. Both retain the Session and historical artifacts without a separate upgrade or binding ledger.

`knowledge.management` uses the existing domain service for list, inspect, managed creation, metadata/URL-source updates and deletion. Partial creation returns the committed base ID and actual document/source status, so retries repair that base. Mounting uses the existing knowledge selector at an idle boundary; a tool cannot rewrite the running Turn's frozen binding. `session.management` queries and creates canonical Sessions, sends input to another idle ordinary local conversation, cancels an exact observed Turn, and manages real PTYs and linked AgentExecutions. Input uses canonical idempotency and binding/context fences. Terminal input/relaunch/stop check owner, row version and PTY epoch; Execution mutations require the active lead/attempt relation and exact versions. Target results and terminal output are data, not borrowed authority or permission to replay unknown effects. Effects settle through the existing canonical chain.

Companion persona and historical memory never advertise retired `nomi_*` tools or unverified full capabilities. Current Runtime tool descriptions and real receipts determine names, inputs, target scope and completion. Authorized catalog search finds deferred tools; their absence from the initial schema display does not prove a missing capability.
Canonical Runtime persona/instructions come from this admission's exact frozen Revision
payload. Product persona Context and current facts are assembled separately; old extra
memory snapshots are not restored as static model instructions.

Desktop Companion and Robot are concurrent roles of one Companion identity, with independent
Agent/model defaults and canonical Sessions. Device reconnects keep the device Session; browser navigation
does not change its receive route. The lean policy requires `companion`, `companion.memory` and `robot`,
optionally with discovery, delegate-only collaboration, and only source-scoped execution status,
cancellation and question-answer management Actions.
Adding other business capabilities restores ordinary budgets; display names cannot silently narrow an extended Agent.
The derived model context is bounded to 128 KiB, with at most four complete historical user turns, 64 KiB and
16 historical messages. Oversized historical turns are omitted whole, retaining tool call/result pairs in kept
turns; the accepted current input is never truncated. Canonical history and history APIs remain complete.
Cross-Agent handoffs are loaded verbatim up to 16 KiB; larger handoffs receive a short model-projection notice while their canonical facts remain intact. JSON is not truncated and old tasks are not inferred or inherited.
Each turn permits eight model steps without execution-window renewal. Explicit Session reasoning choices remain unchanged.
With no explicit choice, a canonical lean Robot can attach a transient `ResponsiveRobot` preference to its existing
Turn model-configuration pin. Only exact documented provider/model/protocol effort support permits `minimal` or `low`
on that actual attempt. Unknown models, gateways, native thinking controls and lighter settings retain their configuration;
desktop workers use their own model settings. No Session field, saved model setting or frozen authority is rewritten.
Within the existing 24 KiB/initial-definition budget, retain directed delegation and source-task controls first,
then small representatives of granted Robot Actions before richer definitions. Initially visible definitions remain
non-deferred and cannot be evicted by later ToolSearch windows. Oversized definitions remain discoverable;
schemas, resource authority, connection leases and dispatch-time permission checks are unchanged.

Clear physical commands use their matching device tools directly and default to no public reply once every requested
action has a successful receipt. Questions, explicitly requested receipts, failures, uncertainty and necessary clarification
still get a reply. Ordinary companionship follows the requested depth. Missing platform work, including
a short real-time weather query, uses the same Companion's saved desktop Agent through
`strategy=planned,target_role=desktop`; this single-target path skips the planning model. Deep research and elaborate
motion design can use that worker's reasoning, returning a concise answer or a plan for the original Robot to execute
within the original user request and its own device grants. Parent Turns end after acceptance without polling;
typed result Turns return to the original valid device connection. Current live Persona and Runtime guidance apply to
already-authorized device Sessions; seed instructions do not overwrite saved authorship or widen existing Snapshots.

The optional `finish_without_reply` Runtime control declares that the entire physical interaction needs no public answer.
It can follow the final motion/display batch, with real execution still serialized. Host settlement, cancellation and the
atomic terminal-input fence must pass before the typed `silent_completion` observation is committed. An exact completed
Turn with that intent and its empty closing seal derives `AgentTurnOutcome::Silent`; old empty answers remain unavailable.
This is reply intent, never evidence that physical movement or delegated work finished. UI history retains tool activity
without a manufactured answer, while Robot receives only `Done` and does not start TTS. UI/API contract version 37 requires
matching frontend and backend builds. Other Agent empty-answer and delivery guarantees stay unchanged.

Robot ASR runs as an owned task, with at most one recognition in flight and two queued recordings per connection.
The device reader continues handling MCP replies, abort, disconnect and ping. Abort clears unsubmitted audio
and joins recognition cleanup. Replaced connections and host shutdown cannot submit late transcripts as new
Companion inputs. Independent recognition scheduling retains the existing final-public-answer speech contract.

The conversation list's `archived` field is a canonical metadata annotation. It preserves
history, working context, active tasks and ingress routes. It is separate from automatic
Companion window summaries, their fenced context reset, explicit context clear and deletion.
Listing and ensure read the current annotation from canonical metadata rather than an old
navigation-table field; archiving does not redirect incoming device or quick-window input.

The App's compilation surface remains desktop regardless of optional Browser/Computer adapters. Deferring physical availability in the resource ceiling requires an exact pinned bundled public Provider with one member, one resource kind, complete Action operation contracts, and no private implementation, dependencies, Context or Events. Unsupported physical members enter only the current compiled object's non-executable set, never Snapshot or checkpoint data; binding and invocation still reject them. Other Providers, incomplete contracts and direct non-Agent operations retain strict admission.

The authorized catalog is not truncated at 128 Modules or Actions. With ToolSearch, initial schema presentation projects within 24 KiB and 110 tools, reserving Runtime controls and five latest search matches while preserving every binding and schema in the original ToolPlan. Later searches protect the latest set of up to five matches, fill remaining space with older displayed tools, and leave omitted tools searchable. Explicit Host preactivation uses the final authorized frozen names and atomically presents the entire requested set; it does not split them into five-match searches that can evict earlier members. Both latest search results and explicit Host sets may exceed the soft 24 KiB target. The hard limits remain 115 authorized definitions, 128 visible tools including Runtime controls, and the whole-request context budget; an oversized set is rejected without partially changing activation. Canonical recovery rebuilds presentation from its original facts and applies the corresponding set rules. Without ToolSearch, only the actual assembled visible tool count is limited. This window is presentation state, not a second authority set or persistent ledger.

### Relocating the same current dataset

A storage-location change preserves the installation owner, storage generation, Session/Turn identities, encryption
key and current history. The dataset coordinator copies and validates offline after the old services exit and while
holding the source and destination locks. Location revision is distinct from data generation; relocation grants no
clean-cut, destructive-startup rebuild or retired Agent import authority.

New default Workspace and Process Session resources use the typed `workspace_location=managed-session` reference.
The host resolves it only from the active work root and exact canonical Session UUID to `conversations/<Session ID>`.
Explicit external projects retain their absolute roots; a path prefix cannot establish managed ownership. Existing
current bindings become logical references only when the exact `default-workspace`/`managed-process-session` identity
and sole `workspace_root` parameter match the source work root. Managed knowledge roots require the current
`knowledge_bases.managed` flag and exact registered root before their frozen `knowledge_root` changes; external
knowledge roots remain unchanged.

The hosting owner's pure offline registry validation supplies the application UUIDs
for this relocation. The canonical writer also verifies the real installation owner
and accepts only each registered `{source_data}/app-hosting/apps/{app_id}` root.
Inspection never initializes HostingOwner or reconciles orphan processes; any
unconfirmed process identity, PID, running state or operation blocks relocation.
After the full copy is prepared, only `record.directory` in each staging registry
changes to the final destination root. The publication coordinator compares every
source registry with that exact location-only change and preserves all other fields;
application source and other payloads still pass complete size and checksum validation.
The final installed manifest includes rebased registries, so publication recovery
does not replay relocation or obtain authority by reopening the old drive.
Preparation retries accept either the exact original registry copy or its directory-only
rewrite; publication still requires all registries to be rebased. Once a verified copy
exists, retries do not reopen source SQLite and create WAL/SHM sidecars. Complete source
checksums, pure App inspection and staging database validation remain mandatory.

One canonical Store write transaction updates the complete affected Session resource sets, derives new definition
IDs and appends `session/agent-binding-changed` with a typed `workspace_relocation` proof. The proof contains the source
data/work roots, target data root, managed knowledge IDs, optional `managed_app_workspace`
application/owner proof, and complete before/after resource definitions. Only managed
location parameters may change; owner, resource identity, grants, connection and all other parameters remain exact.
This explicit location transition retains the same Revision/Snapshot and increments binding version. A same-Snapshot
transition requires its exact proof and request digest; ordinary Agent/capability changes receive no such exception.
Every location change records the Session-owned `conversations/<Session ID>` namespace, including unchanged logical
resources and Sessions whose current Agent uses an external project. Optional `managed_session_namespace` references
only the exact default Workspace definition in this Session's opening event; the writer and reader verify that origin
and the complete location chain. A selected Workspace/Process root changes only when it exactly names that owned
Session directory. Registered applications use their own `managed_app_workspace` proof
to update Workspace, Process Session and scoped project-memory location parameters
at the exact application root. Their logical resource IDs, grants and connections stay
unchanged; new Sessions in that project reuse the relocated memory identity.
Unregistered, adjacent and external project roots remain unchanged. The entire source
work root cannot be adopted. Authenticated canonical history responses attach transient `managed_file_paths` using the original
source sequence, managed interval, exact Session root and canonical relative components. New explicit references to
old locations, other Sessions and external projects remain unchanged. Timestamp-cleaned attachment names resolve
only through the same accepted input's exact `files` receipts, without disk searches or historical text rewrites.
Ordinary Sessions retained after application deletion grant no authority to relocate or
recreate that application directory. After validating the complete location proof and
exact current definitions, history treats a proved App root outside the active dataset
as external and supplies no new App aliases. Corrupt proofs or current definitions still
fail closed; default managed Session namespaces must still match the active data root.
Image, attachment and tool-file previews/opening share this map and remain usable after the old location is offline.
Existing events, payloads, immutable Snapshots and historical resource definitions stay unchanged. There is no second
binding ledger; a retry verifies the original canonical transition and current binding directly.

Pre-copy source and pre-publication destination checks reject active/paused Turns, Attempts, recoverable native
checkpoints, pending/unknown effects and patch recovery. A server lock or terminal Turn does not prove descendant
exit. Offline checks also read exact-Turn canonical dispatch, owner settlement, `host_process_quiescent` and
`host_cleanup_proven` receipts. Missing cleanup proof rejects relocation without guessed PID kills or authority from
the retired process registry. History readers treat a verified location transition as a data-history boundary, retaining
closed content without native replay or completion-ledger inheritance. After target validation the coordinator commits
one active-root pointer; post-commit recovery finishes that target rather than reopening the old copy automatically.

SDK retries of a published terminal acknowledge the existing receipt only when its original root, delivery,
Snapshot/route and canonical Runtime terminal match, then continue native final release without another terminal or
an altered outcome. Generation-zero unstarted cancellation uses its independent exact witness. Native `turn/paused`
retains the active Operation and checkpoint resume authority, so it is not a final-terminal witness for user rebuild
or duplicate final-terminal acknowledgement. Complete Runtime
teardown retires the exact resource-context instance from the host Weak cache; retained old handles cannot revive
closed contexts, and late old cleanup cannot release or evict successors.

## Built-in browser and Agent Browser authority

The Browser domain owns the user side browser for the same authenticated owner and canonical Session. User navigation
requires no Agent Browser Module or resource binding. Opening and retrying never mutate canonical Agent authority.
Authorized managed Agent wrappers borrow the user's real page while enforcing the frozen Snapshot, exact Provider,
typed resource and BrowserRunGuard. Attached Chrome remains a separate connected resource.

Turn preparation freezes Browser authority without locking the managed browser. Each actual managed browser
invocation acquires the current Workspace's native input gate and guard for its browser work. After native work,
pressed-input cleanup and any applicable Effect result recording settle, the invocation releases ownership while
the same Turn may continue thinking or using unrelated tools. A pending browser dialog or other retained native work
keeps the gate until safe completion. Failed acquisition, uncertain cleanup and Effect-result persistence failure
retain the original recovery owner; cancellation or a dropped caller is not completion proof. Normal local release
does not require a durable Turn terminal and does not mutate canonical Turn state or Effect facts.
A committed same-Turn human wait can settle remaining Browser ownership: the original Store verifies the exact owner,
active Operation, `waiting` outcome and absence of pending/unknown Effects before and after settlement. Original
process owners and deadlines remain. The next Browser invocation binds the current Workspace generation under the
frozen grants; native target, document and observation checks continue to reject stale references. Terminal cleanup
still drains retained work and retries failed releases. User APIs use the actual coordinator gate, without deriving
ownership from Session running or paused, model prose or configured Browser permissions. Chat, file work, attached
Chrome and isolated background rendering leave the managed browser available whenever it has no active browser owner.
User profiles use owner/Session identity independent of grant definitions: Windows uses
`browser-v4/agent-sessions/<hash>/`; macOS 14+ uses a separate persistent
WKWebsiteDataStore with a stable derived identifier and does not read former directory profiles. Session deletion includes this browser and its
profile even when no Agent Browser binding exists; there is no second active Agent authority ledger.
See the [browser architecture](browser-platform.md).

## Global extensions and composer selection

Skills and MCP are shared installation capabilities without a preset master gate. Creation or an explicit
idle update captures the global Skill inventory and compiles selected MCP tools into the same
Session-only Revision/Snapshot. Variants use the existing Agent Store, with no second binding or
permission ledger. Library locks freeze bodies, supporting resources, sources, and digests; `selected`
controls default input injection. Current Package locks retain their exact JSON contract.

The composer uses typed `session_capabilities` and versioned capability-selection commands instead of
Skill or MCP `extra` mirrors. Idle updates reuse canonical binding transitions, preserve non-MCP
resources, prove Runtime teardown and settled effects, then commit binding, resource definitions, and
active set atomically. Active or paused Turns, Remote Sessions, and Attempts cannot update selection.
Extension changes use the same frozen role Provider resolution as ordinary execution; selecting Skills or MCP
does not adopt unrelated installation role defaults. An explicit update compiles the requested final configuration
once and proves that only its authorized extension selections and forward bundled identities changed. The remaining
Agent authority, dependency closure and Provider mounts stay frozen. The old complete extension selection is not
recompiled first, so removing a withdrawn MCP tool or Package Skill does not require that old extension to remain available.
The owner commits one canonical transition after the same idle, cleanup and effect checks.
Bodies and resources use the same native frozen context reader; Skill hooks, shell, and forks do not
execute or grant tools. Tool search is a built-in Runtime facility.

## Consumers and deletion

UI consumes current stream and Message projections directly. It has no marker-dependent local terminal-processing state machine or prose-based reclassification of old errors. Channel binds through its current owner; orphan rows without an authority binding produce a conflict instead of being automatically rebound.

### Canonical process timeline

Live and historical display share a read-only timeline derived from canonical content-part and typed Runtime tool events. Existing content writers, seals and `agent_messages` remain the content index. Narration ownership and reasoning activity have independent lifecycles: reasoning deltas, completion and signatures never split one text source. Transport packetization, delta kinds, character counts and punctuation do not define paragraphs. Blocks reference exact UTF-16 half-open source ranges or the original tool call, without another body, phase table or checkpoint. The first canonical content event identifies a block; the accepted root or preceding real narration boundary anchors its process group. Distinct text sources, model steps, tool proposals/calls, accepted steering, withdrawal and reviewed delivery retain their boundaries. Late tool results update their original group.

The Store returns source content and timeline references in one read snapshot with `through_seq`, preserving the raw pagination cursor. Partial Turns retain their real group anchors without loading the entire Turn. Live text frames atomically carry their source's canonical timeline; a later, best-effort notification cannot establish a new text boundary. If the exact display snapshot is temporarily unavailable, a Turn-local queue retains unpublished text within the existing stream budget and releases it with the recovered boundary in order, without rewriting canonical content or converting a display failure into an execution failure. Ordinary uncommitted tails reuse the same source's confirmed range. The separate `canonical_timeline` notification only updates existing rows in its exact Turn; it cannot raise busy state or create another terminal. Display refresh follows actual buffered commits, while uncommitted token tails remain ephemeral. Recovery preserves the existing bounded content policy and voice withdrawal boundaries. Both pause notifications and authoritative pause reconciliation invalidate canonical history to recover any deferred text; this content refresh neither announces completion nor releases the paused Turn or its input queue.

Optional `ChatOutputTextSource` metadata preserves validated provider text item, output position and content-block identities through the existing Runtime event and `message/content-part.source`. Distinct Responses and Anthropic text blocks remain distinct. Protocols without native block identities use the existing canonical model-step message source. Existing current-generation events without this metadata derive continuous narration from their original message identities and real tool/step boundaries; reads do not backfill identities, rewrite history or create another content table. Runtime content assembly follows the same source contract: ordinary unsigned reasoning does not split its text, while signed, opaque, provider-reasoning and tool blocks remain native barriers with their original ordering and continuation state.

Each text source and reasoning activity commits its first real fragment before live publication so the existing UTF-16 reference identifies its active tail. Reasoning does not reopen narration. Later fragments retain the bounded 16KiB buffering policy, and a batch cannot mix text sources. A checkpoint flush does not reopen the source, and late tool settlement does not split it. Completion reports still publish atomically after their original review and settlement gates, with a separate reading boundary.

Process narration and public answers share `ConversationProse` and a Session-scoped ephemeral display cursor. Messages retain their original zero-delay ingress batches and complete received text; copying, result membership, cancellation and Runtime busy state never wait for animation. Live ingress explicitly admits sources for frame-paced presentation, sharing progress across exact canonical source/ranges; history reads do not grant live eligibility or replay a process-to-answer handoff. Each narration block uses one Markdown tree, reused within its presented source revision, revealing only arrived characters and complete syntax ranges. Code blocks and display math keep stable widgets containing their actual received content; other Markdown consumers keep their default renderer. Cursors are neither persisted nor another appended body. Replacement, withdrawal, cancellation, pause, Session changes and reduced-motion settings retain their immediate boundaries.

The existing ToolCall product projection retains capability/action identity, call names, existing path/operation labels and actual status. Live history and conversation exports exclude file bodies, search snippets, write content, patches, stdin, inline scripts and raw tool output. Existing operation rows remain unchanged; no replacement file list, card or interaction is introduced, and a row with no remaining detail has no empty disclosure. Canonical model arguments and observations are unaffected. Typed process outcomes, permission requirements, bounded-search results, knowledge-write notices and produced media are extracted before product-body removal and continue through their existing product paths. The renderer does not reconstruct these states from arbitrary output JSON. The unused `content_mode` query option and business-ignored knowledge-write `title` are retired. UI/API contract version 37 requires matching frontend/backend builds instead of allowing an old renderer to infer status from retired bodies.

`ToolCompleted.runtime_hold` records only a bounded Runtime reason why one proposal did not reach its owner port. `ToolStarted` remains write-ahead admission intent. Discovery, held proposals, owner failures, cancellation and unknown Effects keep distinct semantics. Missing historical reasons remain unclassified rather than guessed from English output. Host settlement remains readable when cancellation prevents a Runtime completion. Display metadata never changes authority, work accounting, delivery review or replay decisions. New reports use the v5 public format without automatically appended aggregate failure counts. Work accounting and delivery gates retain their original values; saved current-generation older deliveries keep exact text validation without rewriting history.

All shared conversation surfaces use the same process structure. Consecutive reasoning and tool activity form groups separated by narration; public result membership remains owned by `AgentTurnOutcome`. Tool titles are plain transparent rows; only expanded details have a subtle background, with no decorative border, leading rail, nested indentation or aggregate exception count. Process groups start collapsed, and terminal Turns default to hiding the overall process. Stream increments preserve manual disclosure and reading position. Closing group membership does not prove parallel tools settled. Permission actions, deliverables and user questions retain their existing access and result contracts.

Only outer group titles pulse while processing, independently of expansion; detail rows never pulse. Within an open Turn, groups with running members and the trailing group before any later narration or group retain title activity, including model-response and argument-generation waits. Ordinary model status rows do not end that continuation. This derives presentation activity without changing tool outcomes, canonical segment states or expansion defaults. Turn settlement stops the pulse immediately, and reduced-motion preferences disable it.

Error diagnostics capture the Agent, model and workspace admitted for the exact Turn and retain them with the original detail in the existing canonical terminal payload. The backend emits typed `taskIncompleteReason` for incomplete tasks; the renderer never guesses the cause from diagnostic prose. Live errors and historical projections consume the same terminal information. Missing historical fields remain unrecorded rather than borrowing the Session's current selection. User-facing reasons and recommended actions are separate from collapsed technical details.

One `ModelFailureDiagnostic` carries model-request refinements and evidence from invocation through the Broker, typed failure and existing terminal into `providerDiagnostic`; classification is never reconstructed from English messages, UI projections or unknown response bodies. Explicit upstream machine codes take precedence over broad status. When HTTP 401, 403, 404 or 429 alone cannot prove credential expiration, model access, an incorrect API path or depleted balance, the explanation retains that uncertainty. Diagnostics capture the sanitized actual endpoint, provider and model, protocol/auth method, HTTP status, upstream code/type/parameter, request ID and retry hint; arbitrary response prose, credentials and authentication-header values are excluded. Safe local transport causes remain available in technical details, without changing retry, authorization or settlement rules. Failed-attempt identity after failover is shown separately from the Session-selected model; account links require trusted configuration matching the recorded provider identity.

Failures, current pauses and rejected requests share one error presentation. Model or provider failures automatically unlock the composer after the existing owner proves cleanup and native resource release and commits canonical `turn/failed`; the user does not need to end the Turn again. The same owner may settle an already-saved current-generation typed model failure pause during startup, Session reads or the next admission. Its transaction rechecks the exact pause revision, execution fence, Snapshot, current-generation cleanup receipt and absence of pending/unknown effects. It never runs a model, replays a tool, or changes failure into success or cancellation. Unproven cleanup, unknown effects, manual pauses and other recovery pauses retain their native fences and explicit recovery authority. Genuine pause notes derive only from verified current state and never enter Message history or invent failed terminals. Rejected requests remain transient within their originating Session, and an admitted request's canonical error replaces its HTTP fallback.

Cron, Companion, Requirements, AutoWork, IDMM and AgentExecution retain their own business configuration and supervision state. Session input, admission, cancellation and completion use canonical Turn receipts.

An `auto_group` channel guest enters a dedicated model-only Session through the
trusted Channel port and repository relationships. A caller-supplied
extra grants no guest or product authority. The final frozen Snapshot has no
business capabilities, Context, typed resources, Skills or MCP. Previously broad
guest Sessions narrow only at idle CAS/effect/native-cleanup boundaries and then
formally clear old context. Warmup, native recovery and new input verify that same
actual contract. Approved channels retain their existing Agent/resource selections.
See the [Companion acceptance status](companion.zh.md) for this integration's validation.
Group reuse locates the exact Session by its formal creation key; only an explicit
send repairs a stale receive pointer. Guest receipt replay requires the original
accepted-admission Snapshot to equal the verified current empty Snapshot, with its
root after the formal clear. Old broad receipts and old keys across model/contract
changes are not forwarded; canonical history remains. The Host fixes the reserved
`channel_guest` input origin, ordinary APIs reject impersonation, and collection
excludes those inputs from the owner's ambient learning.
The trusted guest Host request permits one chat-only model step, without continuation
segments or HistoryPort, and disables UserInput. Provider-invented tool proposals are
rejected and recorded by the existing exposure guard, without a second corrective
model call or adaptive ToolHistory LOAD/SEARCH/READ. Approved channels and ordinary
minimal Agents keep their existing contracts.

The two Companion product roles select the official Companion/Robot templates or
compatible personal Agents. Compatibility requires the Companion identity and memory
core and the necessary resources. The templates have different defaults, while the
workbench exposes the full capability catalog for both. Explicitly selected physical
and local-work capabilities follow normal authorization and resource contracts without
role-based clipping during save, creation or application. Resource resolution uses the exact target
companion and cannot inherit another companion's identity or memory resources.
The owner-scoped canonical opening fact identifies the Companion product
Session; typed origin and binding together establish companion, role and device ownership regardless
of whether the selected Agent is an official template. A regular Agent Session
that mounts a Companion resource does not become that companion's product
Session. Desktop, Channel and Robot may use independent canonical Sessions; viewing
a Session never changes an ingress receive target.

Voice from a paired robot enters its dedicated Robot Session. Admission holds the
current connection fence and verifies the exact canonical Companion, role and
device. The authenticated host records typed input provenance; ordinary requests
cannot invent it through an origin string. Camera permission and robot vision are
independent. Voice input does not alter frozen grants; tools retain resource,
Turn, connection and device-permission checks.

Directed desktop results first commit a canonical report with the original
Execution/Step/Attempt/Turn references, then admit an idempotent follow-up Turn at
the original Robot Session's idle boundary using typed `DirectedDelegationResult`.
Ordinary question notices reference the original `DecisionRequested` and verified
presentation. Physical audio captures the unique already-presented question at the
recording/ASR queue boundary and carries it through recognition and input admission.
A new question may play while older audio awaits ASR, but that transcript retains
its original `None` or older question ID and cannot acquire the newer question.
Replaying an accepted old speech source only acknowledges it without rolling back
the current question cut. The answer Action reads the accepted root rather than
model-authored answer parameters. Successor questions, multiple pending questions and
click-required approvals cannot gain authority through that Action. Completed parent
Turns use the Store's exact read-only `turn_accepted_input` query, not native recovery
reads restricted to an active Turn.
An explicitly failed result Turn produces one idempotent fixed Host notice tied to
its original report and failed terminal, on the original socket. A cancelled result
Turn stays stopped without automatic failure playback or another model Turn.

Same-companion facts refresh before subsequent model calls, and bounded recent context
comes from typed canonical events. Deleting or correcting long-term memory invalidates
later fact reads and source-traceable sharing/recall. The original Session conversation,
local summaries and already-sent model requests remain. Use formal context clear to stop
using that working context, or canonical Session deletion to remove its records.

An ordinary explicitly configured Persona Agent may also refresh bounded facts before
each model call. Its frozen bundled `companion` BeforeTurn Context must be admitted
and active, with the frozen `companion.memory/recall` grant and exact same-owner,
same-companion `companion/read` and `companion_memory/read` resource pair. This does
not create product Session membership or automatically load sibling conversation text.
Explicit history recall retains its separate frozen tool/resource authorization and
verifies the target canonical product sources of that companion; its caller need not
be a product Session. Automatic
recent context honors every formal context clear. Explicit history retrieval may
cross only fully verified Host automatic-window-summary clear facts; explicit user
clears and unknown old shapes remain hard boundaries.

Installed memory Actions use Wave 1's formal schema, validation and typed DTOs to
reach the unified Companion service. Wave 4 registers only learn/evolve, without a
second memory-write entry. Facts and history queries both accept at most
1024 Unicode characters. The Host derives typed message refs from the current tool's
canonical human root/steering causation, including formal `mobile_voice` inputs;
models cannot supply provenance. The Companion Store saves those sources atomically
with creation, deduplication or CAS correction. Source filters cover both the original
Session column and typed refs. Existing domain idempotency receipts retain their old
fingerprints and use `HostedEffectReceipts` to settle the same Effect. Unknown outcomes
are not replayed, and old receipts without an Effect do not fabricate past execution.
Memory tools use one `AgentMemoryProjection` for model output and budgeting, omitting
the two full source-lineage arrays while storage, REST and exports retain them. Default
recall, queries, and fresh or historical write receipts use that same projection.

Historical Directed dependencies and known-receipt replay use the Store's read-only
`committed_tool_call_causation` to verify exact typed tool facts on closed Turns. This
does not widen active/native admission or grant new execution authority.

Existing visual observations are optional context. When the device disables
vision permission, the host omits that context without reading cached images;
authorized display and other device tools remain available. Explicit photo
requests still check vision permission when invoked. Robot resource operations
come only from the exact frozen Action selection; display or device Actions
cannot grant vision authority.

A Runtime that materialized a robot tool catalog uses the existing turn-boundary
recycle after a fully closed Turn: it proves that the old Runtime exited and
effects settled, then rebuilds local resources for the next Turn against the
current device connection. A prewarmed instance that has not started a Turn
also rematerializes at first-input admission, so a reconnect cannot retain its
old catalog. Empty catalogs refresh too. Sessions without a materialized robot
catalog keep ordinary Runtime reuse. This preserves the Session, canonical
history, Agent, model and resource authority. Paused Turns do not enter this
path; unknown effects and incomplete cleanup still block a successor. Each new
device Turn currently incurs one local Runtime rebuild, without permitting old
connection calls or replaying old operations.

An explicit `conversation_id` and resource selections at the product Agent binding
endpoint apply to that exact Session of the target role. Saving future defaults also
requires `update_default=true`; a request without a Session defaults to updating the
role's future configuration. Only a default update with `model` materializes its future
binding; omitting `model` preserves the `needs_model` contract. Even when Agent identity is unchanged, the host
compares resolved resources and uses the idle ContextOnly / CAS transition,
preserving Session ID, current model and Companion identity. An identical
selection appends no transition. Failure to apply the current Session does not
first save its future default configuration. Other existing Sessions keep their bindings.

Model and extension derivation may bind execution to a hidden Session-only
Preset. That cache ID cannot replace the user's original editable Agent choice.
The selector proves the original selection command against the existing typed
Agent transition's `request_digest`, without name/configuration inference or a
second source-binding ledger. The product selection saved before initial
creation remains authoring intent. Editing a personal Agent requires an explicit
application of its latest setup before the current Session's frozen version changes.

Ordinary, Companion and read-only conversations share `NomiChat`, deriving current
Agent identity from the complete conversation projection. The shared history reader
refreshes committed boundaries on `agentSession.agentChanged` for its own Session.
Current and historical official names use host-verified template provenance,
including product Agents such as Companion; ordinary chat selection filters never
limit name presentation. Personal Agents retain their frozen names.

Saving a Companion selection validates target ownership, the current binding
version, model and capability availability, complete resources and runtime
cleanup under the same Session owner's operation fence. An existing idle
Session changes through the existing `session/agent-binding-changed` transaction,
preserving its identity, current model, selected extensions and still-applicable
resources. Previous Revisions, Snapshots and events remain immutable. The
product default supplies subsequent Session creation; it neither replaces the
current Session's canonical binding nor lets a UI projection claim application.
If default persistence fails after a committed transition, the API reports that
the current setup is already active and permits an explicit retry of the default.
Active or paused Turns, Remote and Attempt bindings, unsettled effects, running
processes and pending recovery retain their switch fences. The transition never
replays a checkpoint, tool call or completion ledger.

IDMM explanations are typed metadata on canonical accepted input. Only the
trusted IDMM command can supply `idmm_decision`; public input cannot claim the
IDMM source or supply reserved annotation fields. The immutable explanation
records the actual source, bypass model when used, short basis and exact original
question reference. Live UI events and historical Message projections use this
same committed fact. Neither a configured model nor an old origin field or audit
entry can supply missing explanations.

Automatic answers require a transactionally verified current question and ready
Session. A waiting-for-human or failed decision is recorded as
`idmm/notice-recorded`, validated against the exact question and projected as a
conversation notice. It creates no Turn and does not enter Runtime context.
Decision explanations also remain outside model message bodies. The UI display
preference controls expansion only; it invokes no model and carries no confidence
percentage.

AgentExecution settlement, restart recovery and manual adoption use one Session output query for the exact Turn's terminal receipt, assistant content and settled tool effects. The query resolves only that Turn's event window in one transaction rather than scanning the whole Session history. Content cannot come from UI projections, reasoning or later turns. File outputs require successful file-operation or publication receipts and verification of workspace identity, path, byte count and digest. Directory scans, model claims and retired display markers are not delivery evidence. Later writes, patches and deletes determine the final output state in event order.

The conversation file list reads the exact accepted root's closed Turn and combines settled file-owner receipts, publication receipts, and `turn/workspace-files-observed` presentation observations. It does not infer files from paginated messages, tool names, arguments, or prose. The file domain captures bounded start/end fingerprints inside the existing resource lifecycle, including managed non-Git workspaces, without writing a Git index, creating a shadow repository, or retaining per-Turn file contents. An observation stores normalized relative paths, root identity, net changes, and coverage in the same Event/Payload chain; the native lease validates Session, Operation, generation, and admission predecessor. Display metadata has an inline byte budget; truncation reports incomplete coverage without consuming the native stored-Payload budget. Recovery captures a real new segment baseline and retains earlier confirmed observations and receipts; a missing original baseline reports incomplete coverage instead of reconstructing the past from current disk state. Older plain-chat Turns without observations or admitted file work do not produce empty partial cards; older command-only Turns retain partial coverage without guessing historical files.

File observations are excluded before Payload hydration from native history and never enter the native journal, model context, completion evidence, or checkpoint budget. Presentation keeps the original Turn terminal bound without widening execution-output evidence windows. Scan limits and filesystem failures reduce file-list coverage without changing tool results, Turn outcomes, or cleanup proofs. Deleted and subsequently unavailable files remain listed; availability controls existing actions only. Ordinary file actions open the current source, while publication identity and source location remain distinct. Live and historic views use the same query, and relocation reuses canonical managed-path proofs without a second change ledger or rewritten history.

A Step spec is task input rather than a second artifact contract. The sole Runtime enforces typed requirements, completion and delivery; the scheduler cannot infer another acceptance gate from reference filenames, formats or counts in prose. Recovery uses the Session owner's complete canonical OperationId and the same output and error classification as normal settlement. Terminal metadata cannot become an output summary. Missing, unknown or incomplete receipts do not authorize replay; automatic retry requires an explicit structured retry permission.

Automatic tool routing suggests actions for explicit current-input intent without truncating the authorized catalog. Delegated input classifies only step_spec, excluding task_brief and reference content. Tool discovery searches both advertised and deferred authorized tools; exposing a schema earlier does not add authority.

Cancellation fixes its target Turn in the canonical owner's transaction. Runtime cancellation uses only that receipt's target and native execution generation. Replays and delayed StopTurn commands cannot cancel a successor. Execution persists a stable cancellation identity and exact target OperationId in its existing write-ahead intent. Decision continuations share ordinary scheduling jobs so other results, cancellation and lease loss remain responsive.

The refactor uses an Agent-only clean cut in the same database. Retired Agent sessions, messages, logs, Execution, presets and bindings are neither imported nor read. Non-Agent configuration follows its explicit owner and cutover contract. Repository cleanup must not operate on a developer's or user's live data directory.

The destructive upgrade is owned by the application dataset coordinator. A readable historical database outside the current canonical migration chain, including an 87-migration database with v3 identity tables, is retired together with managed side stores through a locked, resumable reset plan. No historical rows or configuration are imported. Files are isolated under `retired-datasets` and never read by the replacement dataset. A previous one-time legacy retirement does not block this upgrade. The new installation owner and storage generation are finalized only after all services bootstrap; subsequent starts preserve the new data. Failed database inspection and damage to a supported schema/data contract still fail closed. More migration rows do not identify a newer application version and must not block the destructive upgrade from 0.8.0/0.8.1. Direct database initialization and backup/restore retain strict validation without startup rebuild authority.

Replacement includes physical deletion of the old reader, writer, DTO, schema fields, fixtures, tests and documents. Published migration lineage needs its own deliberate boundary; checksum rewriting and permanent dual writes are not substitutes for deletion. Unused generic checkpoint APIs, historical deletion manifests and generator requirements for obsolete documents are removed.

## Sources and validation

- [Session owner](../../crates/backend/nomifun-conversation/src/canonical_session_owner.rs) and [Store](../../crates/backend/nomifun-agent-session/src/store.rs)
- [Official Runtime](../../crates/backend/nomifun-app/src/router/official_runtime.rs) and [Runtime implementation](../../crates/backend/nomifun-agent-runtime/src/lib.rs)
- [Journal](../../crates/backend/nomifun-app/src/router/engine_journal.rs), [canonical history](../../crates/backend/nomifun-app/src/router/engine_history.rs) and [native replay](../../crates/backend/nomifun-app/src/router/unified_runtime_history.rs)
- [Contracts](../../crates/backend/nomifun-agent-contracts/src/lib.rs) and [schema](../../crates/backend/nomifun-db/migrations/001_canonical_baseline.sql)
- [Official seed](../../crates/backend/nomifun-agent-contracts/contracts/presets/official-agent-seed-manifest.payload.json) and [recommended-draft owner](../../crates/backend/nomifun-agent-control-plane/src/service.rs)
- [Action descriptors](../../crates/backend/nomifun-agent-contracts/src/package.rs), [compiled resource checks](../../crates/backend/nomifun-agent-kernel/src/compiler.rs), [authority](../../crates/backend/nomifun-agent-kernel/src/authority.rs) and [resource resolver](../../crates/backend/nomifun-app/src/router/nomi_core_resource_bindings.rs)
- Native [knowledge management](../../crates/backend/nomifun-app/src/router/knowledge_management.rs) and [Session management](../../crates/backend/nomifun-app/src/router/session_management.rs)

Run targeted Rust/UI tests, contract generator check, `bun run check:agent-session-boundary` and `bun run check:uarc-boundary`. Renderer changes also require `bun run check:desktop-ui-boundary`; the minimum supported desktop viewport is 880×600.

Cover new and sequential turns, rejected admission followed by a new turn, Agent switches, context clear, cancellation, restart, pause recovery, unknown effects and deletion. Missing structured history must not be repaired by a projection fallback.

Preset completion also covers real first-Companion file writes and artifact receipts, asset/Office Actions without Canvas, other tools remaining available when optional environments are absent, read-only recommendations and SaveRevision CAS, an existing Session's next Turn after applying saved updates, and management owner/version/PTY-epoch fences. Temporary directories and simulated HTTP do not prove external generation services, real devices or production behavior.
