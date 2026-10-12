# Agent Session logs and content architecture

AgentSession is the sole fact chain for sessions, turns, content, effects and cancellation. User-facing Conversation APIs project the same `AgentSessionId`. Chat and Coding run through one in-process `nomifun.nomi` Runtime.

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

Official Agents have stable, owner-scoped editable Presets and use the same immutable revision save path as personal Agents. Release seeds initialize unsaved defaults without overwriting saved customizations. Official product launches resolve the owner's saved configuration; model and extension variants continue through the existing Revision/Snapshot path. Historical official identity is validated from host-owned source, owner, and template identity rather than equality with today's seed or mutable configuration.

`engine_history.rs` reads typed canonical events and resolved payloads. It does not read Message projections to determine model roles. Native closed turns replay their structured journal. Missing Runtime records fail closed rather than falling back to chat text.

An accepted input that fails or is cancelled before Runtime startup remains valid context together with its explicit terminal fact. It does not acquire fabricated Runtime, tool or completion evidence.

Creation prompts, Cron notices and Execution summaries enter context through formal message events. Text before an explicit Agent transition is data only; it does not carry old grants, handles or completion gates. Context-clear events and the fixed accepted-root cursor bound the window.

Current-generation history, immutable Snapshots, model changes and explicit Agent transitions remain supported product semantics. They are verified against identity, sequence, binding and content integrity. Executable recovery requires the native checkpoint's exact build and binding. Closed read-only event history may cross an explicitly validated transition without gaining authority.

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
| `assistant.general` | Files, processes, version control, artifacts, project memory, research, schedules, browser and computer operations, media, assets, Office, plugin development, SSH, knowledge and Session management. |
| `coding.codex` | Files, version control, processes, artifacts, project memory, research, collaboration, plugin development, SSH, knowledge and Session management; retains headless Runtime requirements without browser or desktop-control dependencies. |
| `companion.default` | Keeps persona, long-term memory, robot vision, channels and schedules; adds everyday files, processes, artifacts, research, browser, computer, media, assets, Office, SSH, collaboration and management. |
| `customer-service.default` | Customer notes, handoff, knowledge retrieval, channel replies and sends, files, processes, artifacts, research and schedules; no default global knowledge-registry, cross-Session or terminal management. |
| `creative-studio.default` | Media, Canvas, assets, templates, Office, files, processes, artifacts, project memory, research, browser, collaboration and schedules; retains its three creative Package Skills. |

General and Companion grant only asset read/write in `creative.workshop`; a missing Canvas cannot hide those Actions. Native Office tools edit or preview asset-library documents, sheets and slides. Local-file workflows continue through authorized files/process tools. Global Skills and MCP selections retain their exact compilation path and do not substitute for native Action grants.

Frozen `CapabilityActionDescriptor.required_resource_operations` has explicit optional semantics. `None` or an omitted field retains the complete Module resource gate; sibling Actions cannot imply that an unspecified Action is resource-free. `Some(empty)` explicitly consumes no resource slots, while `Some(map)` requires a binding of each named kind with every declared operation. Model-visible tools and Kernel admission use the same frozen check. Context/Event consumers keep the Module resource check, and Role Providers keep their exact implementation slots. Missing optional targets disable only dependent Actions; filtering never bypasses owner, Snapshot, active-capability or invocation authority.

Ordinary creation and product entry supply deterministic host resources under the target's frozen grants: Workspace, Process Session, project memory, asset library, Scheduler, and applicable Terminal, Browser and Computer resources. Product identity uses the actual Companion, Customer or Canvas target ID; Workspace and Process share one root. Published Browser/Computer Role contracts may be frozen while their optional live environment remains unbound. Knowledge bases, channels, SSH hosts and robots require actual selections, without invented connections or credentials. Owner mismatches, storage I/O or corruption, contract/Provider mismatches, native checkpoint safety and unknown effects retain their fail-closed boundaries.

Saved official customizations are not overwritten by release seeds. The owner-scoped `GET /api/agent-presets/{preset_id}/recommended-capabilities` returns a read-only merged draft and missing Module/Action/Package Skill additions. Persona, instructions, model, Runtime settings, existing extensions and pinned Skill versions remain authored choices. The UI shows additions before saving through the existing SaveRevision `expected_current_revision` CAS and does not replace unsaved edits. Saving leaves existing Sessions unchanged. A Companion's explicit Apply latest command uses the canonical Agent switch at an idle, settled boundary, preparing and committing the current owned configuration under the Session operation and binding-version fences. Direct Session preview/apply additionally requires the exact previewed target Snapshot. Both retain the Session and historical artifacts without a separate upgrade or binding ledger.

`knowledge.management` uses the existing domain service for list, inspect, managed creation, metadata/URL-source updates and deletion. Partial creation returns the committed base ID and actual document/source status, so retries repair that base. Mounting uses the existing knowledge selector at an idle boundary; a tool cannot rewrite the running Turn's frozen binding. `session.management` queries and creates canonical Sessions, sends input to another idle ordinary local conversation, cancels an exact observed Turn, and manages real PTYs and linked AgentExecutions. Input uses canonical idempotency and binding/context fences. Terminal input/relaunch/stop check owner, row version and PTY epoch; Execution mutations require the active lead/attempt relation and exact versions. Target results and terminal output are data, not borrowed authority or permission to replay unknown effects. Effects settle through the existing canonical chain.

Companion persona and historical memory never advertise retired `nomi_*` tools or unverified full capabilities. Current Runtime tool descriptions and real receipts determine names, inputs, target scope and completion. Authorized catalog search finds deferred tools; their absence from the initial schema display does not prove a missing capability.

The App's compilation surface remains desktop regardless of optional Browser/Computer adapters. Deferring physical availability in the resource ceiling requires an exact pinned bundled public Provider with one member, one resource kind, complete Action operation contracts, and no private implementation, dependencies, Context or Events. Unsupported physical members enter only the current compiled object's non-executable set, never Snapshot or checkpoint data; binding and invocation still reject them. Other Providers, incomplete contracts and direct non-Agent operations retain strict admission.

The authorized catalog is not truncated at 128 Modules or Actions. With ToolSearch, initial schema presentation projects within 24 KiB and 112 tools, reserving Runtime controls and five latest search matches while preserving every binding and schema in the original ToolPlan. Later searches use a window of at most 117 authorized definitions, retain all latest matches, and leave evicted tools searchable. Canonical recovery and preactivation replay the same rules. Without ToolSearch, only the actual assembled visible tool count is limited. This window is presentation state, not a second authority set or persistent ledger.

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

One canonical Store write transaction updates the complete affected Session resource sets, derives new definition
IDs and appends `session/agent-binding-changed` with a typed `workspace_relocation` proof. The proof contains the source
data/work roots, target data root, managed knowledge IDs and complete before/after resource definitions. Only managed
location parameters may change; owner, resource identity, grants, connection and all other parameters remain exact.
This explicit location transition retains the same Revision/Snapshot and increments binding version. A same-Snapshot
transition requires its exact proof and request digest; ordinary Agent/capability changes receive no such exception.
Every location change records the Session-owned `conversations/<Session ID>` namespace, including unchanged logical
resources and Sessions whose current Agent uses an external project. Optional `managed_session_namespace` references
only the exact default Workspace definition in this Session's opening event; the writer and reader verify that origin
and the complete location chain. A selected Workspace/Process root changes only when it exactly names that owned
Session directory, preserving resource identity, grants and connection. The entire source work root or adjacent paths
cannot be adopted. Authenticated canonical history responses attach transient `managed_file_paths` using the original
source sequence, managed interval, exact Session root and canonical relative components. New explicit references to
old locations, other Sessions and external projects remain unchanged. Timestamp-cleaned attachment names resolve
only through the same accepted input's exact `files` receipts, without disk searches or historical text rewrites.
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

Every Agent Turn locks its Session's managed browser during retained Runtime preparation, including chat without
Browser tools. Settlement drains native operations while retaining the hardware input gate. Only the exact durable
Turn terminal permits final release; later cleanup or terminal-write failures leave input locked. User commands and first native
creation fail closed while canonical running precedes a proven gate. User profiles use owner/Session identity independent
of grant definitions: Windows uses `browser-v4/agent-sessions/<hash>/`; macOS 14+ uses a separate persistent
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

Error diagnostics capture the Agent, model and workspace admitted for the exact Turn and retain them with the original detail in the existing canonical terminal payload. The backend emits typed `taskIncompleteReason` for incomplete tasks; the renderer never guesses the cause from diagnostic prose. Live errors and historical projections consume the same terminal information. Missing historical fields remain unrecorded rather than borrowing the Session's current selection. User-facing reasons and recommended actions are separate from collapsed technical details.

One `ModelFailureDiagnostic` carries model-request refinements and evidence from invocation through the Broker, typed failure and existing terminal into `providerDiagnostic`; classification is never reconstructed from English messages, UI projections or unknown response bodies. Explicit upstream machine codes take precedence over broad status. When HTTP 401, 403, 404 or 429 alone cannot prove credential expiration, model access, an incorrect API path or depleted balance, the explanation retains that uncertainty. Diagnostics capture the sanitized actual endpoint, provider and model, protocol/auth method, HTTP status, upstream code/type/parameter, request ID and retry hint; arbitrary response prose, credentials and authentication-header values are excluded. Safe local transport causes remain available in technical details, without changing retry, authorization or settlement rules. Failed-attempt identity after failover is shown separately from the Session-selected model; account links require trusted configuration matching the recorded provider identity.

Failures, current pauses and rejected requests share one error presentation. Model or provider failures automatically unlock the composer after the existing owner proves cleanup and native resource release and commits canonical `turn/failed`; the user does not need to end the Turn again. The same owner may settle an already-saved current-generation typed model failure pause during startup, Session reads or the next admission. Its transaction rechecks the exact pause revision, execution fence, Snapshot, current-generation cleanup receipt and absence of pending/unknown effects. It never runs a model, replays a tool, or changes failure into success or cancellation. Unproven cleanup, unknown effects, manual pauses and other recovery pauses retain their native fences and explicit recovery authority. Genuine pause notes derive only from verified current state and never enter Message history or invent failed terminals. Rejected requests remain transient within their originating Session, and an admitted request's canonical error replaces its HTTP fallback.

Cron, Companion, Requirements, AutoWork, IDMM and AgentExecution retain their own business configuration and supervision state. Session input, admission, cancellation and completion use canonical Turn receipts.

Companion product Agent selections may use the official Companion template or
a compatible personal Agent. Compatibility requires the Companion capability
and required `companion` resource. Resource resolution uses the exact target
companion and cannot inherit another companion's identity or memory resources.
The owner-scoped canonical opening fact identifies the Companion product
Session; its typed binding establishes the exact companion ownership regardless
of whether the selected Agent is an official template. A regular Agent Session
that mounts a Companion resource does not become that companion's product
Session. Desktop, Channel and Robot continue to reference the same current Session.

Model and extension derivation may bind execution to a hidden Session-only
Preset. That cache ID cannot replace the user's original editable Agent choice.
The selector proves the original selection command against the existing typed
Agent transition's `request_digest`, without name/configuration inference or a
second source-binding ledger. The product selection saved before initial
creation remains authoring intent. Editing a personal Agent requires an explicit
application of its latest setup before the current Session's frozen version changes.

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
