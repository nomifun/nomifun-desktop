# Agent Session logs and content architecture

AgentSession is the sole fact chain for sessions, turns, content, effects and cancellation. User-facing Conversation APIs project the same `AgentSessionId`. Chat and Coding run through one in-process `nomifun.nomi` Runtime.

The [Companion implementation plan and acceptance status](companion.zh.md) covers
one Companion Agent, independent entry-point Sessions and bounded live shared memory.
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

Creation distinguishes released templates from saved Agents. A `kind:template` selection uses the current release seed and live capability catalog for admission, resource preview and compilation. It never reads an owner's saved official Revision or inherits defaults from a hidden compilation. The model comes from the explicit selection or current installation default; template capabilities follow the released definition.

Stable owner-scoped official Presets, personal Presets and immutable Revisions remain intact. A `kind:preset` selection uses that saved configuration. Workbench launch preserves its Preset identity, and selectors distinguish saved official customizations from current templates. An untouched official editor without a Revision may prepare the current baseline on first use without requiring an extra save. Saving does not overwrite the seed or existing Sessions; an invalid explicit saved configuration cannot borrow a template's permissions.

Hidden `session_only` configurations are host-owned compilation caches and reject public authoring saves. Reuse requires the requested payload and complete current execution contract to match. Stale candidates are not evolved or rewritten; the current authoritative definition is compiled afresh. Storage I/O and invalid current definitions still fail closed. Internal template preparation retains the exact returned Revision/Snapshot instead of selecting authority again through a mutable latest pointer. Existing Sessions keep their frozen binding. Official lineage is trusted presentation only and cannot prove a current-template selection or suppress an explicit Agent switch. UI/API contract version is 45.

Saved SSH host launch is a Host-owned entry: it prepares an independent hidden configuration from current `chat.minimal` plus the exact `ssh/exec` definition, then checks the real host owner, resources and canonical creation fences. The client supplies only the host identity and model. It does not edit template caches, create visible personal Agents or make ordinary resource selection grant SSH authority.

`engine_history.rs` reads typed canonical events and resolved payloads. It does not read Message projections to determine model roles. Native closed turns replay their structured journal. Missing Runtime records fail closed rather than falling back to chat text.

Tool bodies serve model context, effect verification, recovery and explicit read-only inspection. Within the existing Event/Payload chain, the Store factors formally owned tool arguments, results and their compaction copies into Session-scoped, type-specific content-addressed Payloads. Events retain their individual identities, ordering, owner settlement and Runtime observation semantics. Readers validate the exact slots, ownership, types, lengths and digests in the same transaction before restoring typed content. Append drafts cannot supply the reserved storage-reference metadata; missing or damaged references fail closed. No current file is read to reconstruct historical bytes, and no content table, alternate journal or recovery ledger is added. Unique content counts against the existing Payload budget; expanded content and journal accounting retain their original bounds. Exact native build, binding and authority fences remain unchanged.

Assistant text, thinking and tool content events share the canonical Payload encoding boundary: JSON up to 64 KiB is inline, and larger permitted bodies are committed to the existing `agent_payloads` with their Stored references in the same transaction. The 1 MiB single-Payload limit remains. A provider's longer text fragment must not be lost through an inline-only content-event branch, and providers need not split otherwise permitted fragments to bypass storage wiring.

The shared canonical append entry keeps asynchronous validation state on the heap, so Session creation, Turn and Effect callers do not embed the complete validator state through every caller and exhaust the default thread stack. Validation still borrows the caller's original transaction with the same order and rules; it does not spawn a background writer or change transaction or cancellation boundaries.

Turn and Effect indexes retain lifecycle state and the exact `terminal_event_id`, without a second terminal body. Their readers validate the original start/terminal identities, state and index boundaries and resolve canonical content in the same transaction. Payloads require a database body; there is no unimplemented object-directory mode. Forward migrations remove only redundant columns and preserve every payload identity, body byte and event reference when replacing the table. Unreadable object rows reject migration. The published baseline and migration checksums remain unchanged; current Sessions are preserved and retired Agent data is never converted.

### Original Turn settlement ownership

Run exit, admitted-operation settlement, canonical Turn closure and resource retirement require separate proof. The SDK retains the exact original outcome, input, first failure and live owner state until the original Session owner finishes settlement. The existing Event/Payload chain records `runtime/native-settlement-requested` and its matching `runtime/native-settlement-settled`; these records are custody and settlement facts, never a new task, transcript, retry ledger or success claim. Lost acknowledgements are confirmed by original stable identities without repeating models, tools or delegation.

A paused producer cannot admit new work. Store-issued settlement authority revalidates the exact Session/Turn, generation, pause revision, original operation and control fences on each write, allowing only original observations, cleanup proof and the original exit. Stop and successors cannot be overwritten. A real user Pause remains paused after cleanup; settlement does not grant continuation or model capacity. Its supervisor never joins its own SDK task, independent children, or knowledge writeback waiting for the main terminal.

Tool and MCP owners retain actual returns and pending records rather than permanent failure booleans. Rust task completion, business failure, durable result and native release are distinct. Live consumption and verified recovery share typed progress accounting from the original frozen effect contract, keeping invocation order, budgets, counters, complete result coverage and source identity. Recovery reads exact original Effect results instead of replacing structured delegation receipts with generic success text. Missing proof remains fenced, and checkpoints never recreate live handles.

Typed SQLite Busy/Locked and pool timeouts permit the original writer to retry only its same immutable event, projection or checkpoint, confirming committed writes by original identity. Cancellation preempts backoff without discarding an in-flight database write. Corruption and I/O errors remain closed. A rejected tool-return contract is settled as a trusted error observation on the original call before its pending result is released; invalid content cannot become success or erase unknown Effects and missing cleanup proof.

Native recovery uses `native_recovery_external_reconciliation_required` for effect uncertainty in both same-process handoff and restart recovery; the label does not assert a process restarted. A legitimate immutable uncertain fact already stored under the current schema may retain `process_restart_external_reconciliation_required`. Only reconciliation of that original fact recognizes it, preserving the runtime producer, exact Effect, input digest and causation checks; new admission rejects the old label. Read-side recognition neither rewrites history, imports retired data nor grants business replay.

Parallel read caller futures can still hold Host locks while awaiting admission or a writer. While serially validating and recording a returned result, the Runtime also drives the original batch's remaining callers; it cannot stop polling them and then wait for their locks. Returned values use only temporary batch buffering, native ownership stays with the original Host, and a fast result need not wait for an unrelated slow invocation. The next model request and replay preserve proposal order.

Explicit resume retains continuous ownership from the original queued request through authorization and Runtime attachment. Transient preparation errors preserve that request; revoked receipt authority retires its local waiter. A durable authorization alone is not attachment. After a claim, the owner retains the original Journal before preparation and can settle only through that writer. Once the SDK accepts execution, the outer owner waits for actual attachment or the SDK's canonical exit proof, without inventing a competing pause after an elapsed-time limit. Settlement diagnostics share one bounded redaction boundary while original business outcomes remain intact.

An unconfirmed original Runtime initialization prefix is retained and settled before changing its pause fence. Pausing cannot make that prefix permanently unwritable. A resumed SDK captures its active generation from the exact Runtime instance during the handoff callback; the first Turn-start sequence cannot substitute for it. Completion notification and automatic resource retirement keep that same original generation.

Warm resume can retain both the SDK and accepted root, so a cached committed exit and its settlement flight also bind that SDK's existing opaque task identity. Only a real send advances this local identity; registry admission release and the same task's settlement retries preserve it. Missing identity cannot authorize a match. An old Pause cannot prove cleanup for a new task, and this local identity grants no execution rights: the canonical writer still validates the original native generation. Handoff passes the original Runtime directly instead of selecting an instance by Session. Cold settlement validates the complete continuous journal while comparing only exits within the saved receipt generation; earlier genuine pauses remain history, and conflicting exits in the original generation remain rejected.

### Attempt input admission and continuous custody

Domain `Running` means the original Session link and reservation committed; it does not prove a model or tool started. First input uses the Attempt's existing `InitialInput` outbox: freeze the task and selections, then commit the complete prepared request with its original link. Latest Step text cannot reconstruct an old request, and the outbox is not another transcript. The same SQLite writer that appends canonical `message/user-accepted` and `turn/started` consumes `AgentExecutionTurnAuthority`, checking owner, valid lease, exact Execution/Step/Attempt and applicable versions, the unique Session link, original operation key and Stop/replacement fences. A separate preflight cannot grant admission. An existing stable key first verifies the complete original input/start pair and only confirms that receipt; stale authority cannot thereby start another execution.

The existing Session lifecycle owns initial-input and accepted parent-question-reply delivery through actual SDK custody or an original canonical exit. A dropped caller, lost commit acknowledgement or transient preparation read cannot end that responsibility. The owner retains the complete prepared request and phase: unaccepted input still needs current admission; accepted input permits only exact receipt verification, observation of the real SDK/settlement owner, or formal native claim of that same Turn. A duplicate Accepted receipt does not authorize another `send_message`. Recovery rechecks custody under its gate, protecting an original SDK that accepted the task while another observer waited, including a slow Driver that has not attached yet.

Session business gates are released before backoff or formal recovery, and no database transaction crosses that wait; user Stop can commit its exact fence first. Progress checks exclude the input task's own gate when deciding whether the SDK took custody. An unexpired old native lease is typed temporary admission waiting, not permanent recovery rejection. After expiry the same claim is rechecked. A first claim without a checkpoint requires the original Turn to prove no admitted execution prefix/effects; checkpoint, build, Snapshot, Unknown and UserPause fences remain intact. Rejected recovery retains explicit reconciliation/pause and settlement through the original writer instead of inventing business failure or safe replay.

The Session input ACK ends only input delivery. The Execution lifecycle still owns the original Step job until Attempt settlement is acknowledged or explicitly handed off. Canonical exit, domain settlement, native cleanup and resource retirement require separate evidence. Controlled Review rechecks original invocation facts and reconciles the same Step/Attempt by CAS. Missing receipt is neither permanently Unknown nor permission to resend an apparently empty Session. Genuine Unknown, corruption, cancellation and user Pause remain protected; [Agent Execution](agent-execution.md) owns scheduling and task controls.

### Child Agents and the original parent Turn

Numbered requirements do not automatically demand raw per-item data or exact-action precommitment. A completion report may omit `delivery_items` while retaining criteria, real evidence and the existing single bounded delivery review. Explicitly selected delivery items retain complete numbered coverage, order and genuine result-reference/value checks. Formatting cannot justify invented references or additional commands.

Each child has an independent canonical Session and Runtime, derived from the original immutable Revision/Snapshot, exact model and typed resource bindings, with the Attempt's explicitly selected subtractive ceiling. A parent's temporary schema window or routing hint never becomes the child's binding. Frozen authorization, Session active capabilities, Attempt restrictions and Turn-visible definitions are checked separately. ToolSearch changes presentation within existing authority; it cannot restore an unselected capability or grant permission.

Attachments and explicitly selected Skills from the accepted source and steering actually applied before delegation pass through the exact delegation Effect, Turn and owner checks into the child's ordinary input preparation. Skill content remains frozen in the child Snapshot; attachments retain the child model's actual input and resource constraints. A followup preserves the original child's history and binding while selecting the sender's exact accepted input and applied steering before its original native command event. It does not reinject the first source's selections or infer them from latest work. The Step spec remains the child's own task input. Original user constraints come from canonical source references, without copying the parent transcript, mirroring tool results or rebuilding context from UI state.

Acceptance leaves the parent Turn responsible for the original task with its complete authorized ToolPlan. A successful receipt does not complete the task or force every later model step to delegate again. The parent reads exact child results with `agent_status`, coordinates owned Steps with `agent_message`, and joins through `agent_wait`. See [Agent Execution](agent-execution.md). Results retain their original Execution/Step/Attempt/Session/Turn/terminal references. The parent integrates and verifies them before ordinary Runtime output delivers the final answer; a separate background-report writer cannot publish a child result as that answer.

A required initial delegation comes only from the current canonical human-intake event and its admitted source, including the original producer, owner, command namespace and message identity. Text or a client-supplied `origin` cannot establish it. Parent background, a child role label and a Host continuation are not fresh human delegation requests. An internal input may expose its current Step envelope only with an Execution source scope verified by the existing reader; that scope grants no Turn or tool authority. Complete tools, concurrency settings and parallel preferences remain available, including explicitly requested nested delegation and further division justified by the current task.

Long child bodies have explicit 32 KiB UTF-8 pages pinned to the immutable source and full-body digest. Only contiguous complete coverage consumes a terminal result; omitted bytes and an isolated final page never count as delivery. Full-scope lightweight Step source headers accompany an observation; filtering bodies cannot acknowledge unreturned bytes. Only a replaced or genuinely changed Step invalidates its old consumption references, preserving unrelated Steps and their page coverage rather than clearing all results on aggregate sequence changes. Native checkpoints retain references, never bodies; fully delivered terminal observations reuse their existing source digest. A newly read child source with real workspace effects or native process ownership invalidates the parent's earlier workspace/completion and scoped-instruction observations once; unrelated domain sequence growth does not.

`agent_wait` records typed `AwaitingChildren` and paired `ChildrenWaitResolved` observations within the same Turn. The native checkpoint retains the operation, model-step and budget accounting, plan, tool observations and wait selection. Crash recovery reattaches that wait without creating children, resubmitting delegation or resetting model budgets. Wake observations are data, not new human input; subsequent status reads verify canonical results. Notifications and display state cannot establish completion. Ordinary queued input retains FIFO, accepted steering joins at the original safe boundary, and a cancellation fence rejects late wakes without borrowing a successor Turn.

The post-checkpoint observation of that same parked wait is a recoverable boundary. Runtime validates its typed tail with the existing progress reducer and retains the original checkpoint unchanged; the canonical claim writer independently pins the full wait request, model counter, checkpoint digest and contiguous original-producer sequence in the lease transaction. A matching event name alone is insufficient. Changed scope, cursor or mode, new model/tool/effect activity, a resolved wait, incompatible build/Snapshot, live resources and Stop or user-pause fences still require their existing reconciliation or control path. Pure waiting cannot authorize a fresh child, replay an original command or grant more budget.

`all` waits for verified outcomes of the selected work, including failure and cancellation, not all-success or proof of parent integration. Explicit, automatic and replayed waits share source-scoped, per-Execution normalization. Omitted `after_cursor` uses the latest successful observation; stale explicit cursors cannot overwrite later trusted journal observations. The compact combined cursor has a 16 KiB bound for up to 64 Executions, without a cache per selection combination. Each reference separates current Attempt/Turn/accepted-command state from exact terminal/question/Review readiness, replacing the unscoped global cursor.

A state change is not always a new decision. When retry removes an old failure from the ready set, the Runtime compares the last successful lightweight source heads and advances only that read subscription. It emits no `ChildrenWaitResolved`, samples no model and preserves result-body obligations; a crash may repeat the read. New or changed outcomes/questions return control once. Accepted parent input and the no-owner decision exit remain separate outcomes and cannot be swallowed by readiness deduplication.

Real waiting requires an existing dispatcher, exact Step/Attempt job, input-delivery task or native execution/settlement owner able to advance the selection. Status text, an unexpired lease or a cache entry alone is not a live-task proof. A different valid boundary is a still-answerable native InputRequired paired with the current exact DecisionRequested: when all remaining unfinished work has that verified question boundary, the original WAIT retains its existing Decision and source-input subscriptions without another model wake. This does not report a job owner or authorize an answer; human presentation, permission and reply checks still apply. A question cannot cover unrelated Review, unsettled delivery or ownerless Running/Queued work. If an observed decision has neither a remaining producer nor that external input boundary, repeated WAIT follows the existing intervention park/pause path, preserving `awaiting_children`, pending results, budgets and controls instead of parking forever or repeatedly sampling the model. Only an actual request for human input creates formal InputRequired; technical Review is not a human question and UserPause is never automatically cleared.

An accepted DecisionInput waiting for native capacity is no longer an unanswered question. Even while the Step remains WaitingInput, both the status read and live wait check its actual existing dispatcher; only that live owner proves progress. The retained outbox alone does not, and the parent's native wait releases its original capacity for that same Attempt's delivery.

Initial plan construction also has a real owner before a scheduler exists. Its existing PlanningGuard is registered before the task's first await and remains held through original result settlement or an acknowledged scheduler handoff; a Planning row alone proves neither. Typed temporary database admission retries only the original read or commit within that owner, retaining a produced plan or definite failure candidate and keeping Stop preemptive. It never retries an entire planner call. After a lost commit acknowledgement, the owner observes the canonical current plan and actually starts or joins its existing scheduler before leaving; successful handoff precedes notification publication. No new planning result, Step, input or execution ledger is manufactured.

Waiting subscribes to the exact owner/Execution and original Turn input notifications before reading canonical state, so a commit during the read remains observable. Original native closure and cleanup acknowledgement still wake existing observers after domain cancellation, without appending another domain outcome or starting work. A 30-second fallback checks domain/native revisions in batches when a notification is missed. Unchanged revisions reuse content observations; changes load narrow per-Execution snapshots and merge new minimal source-proof entries without hydrating Participant Snapshots or launch configuration. Live owners can exit without another event, so handle checks are independent of the content-cache watermark. An unbound Step reservation proves a newly created Attempt only at its exact creation-CAS version. Disposable observations and notifications are neither another fact chain nor delivery receipts.

Pure wait and status reads back off within the original Turn on typed `PoolTimedOut` or SQLite `Busy`/`Locked`, with cancellation preempting the delay. They do not redelegate children, reset budgets or poll through model calls. This does not retry `MESSAGE` writes or treat corruption, I/O, missing facts or permission failures as temporary success. Complete paged consumption and original durable receipts still govern delivery. Database operation diagnostics observe admission, held and finished phases and pool counts without SQLx acquire/release hooks, SQLite calls or another fact ledger.

After a real human-input handoff, a new Turn continues only the task scopes and unconsumed result references established by its valid `reply_to` ancestry. Each hop verifies the original accepted input, exact terminal question, current binding and context-clear fence. A historical question association alone remains data; it cannot grant Session-wide control, and an unrelated new topic cannot take over old collaboration. Readers and command writers share this canonical proof instead of selecting latest work or inferring ownership from text.

Parent management binds the original delegated logical Step rather than its first Attempt. Typed `agent_message.task_control` uses the formal Execution/Step versions returned by status to connect source-authorized retry, recheck, exact stop or replacement with an already completed task. Acceptance freezes command identity and target; retries of that key only confirm the command, and stale guidance cannot silently select a successor Attempt. Legitimate retry keeps the Step and creates a new Attempt. Its accepted intent already represents unfinished work before that Attempt exists, so the old failure cannot satisfy `all`. Replacement retains original result provenance and a formal DAG revision, without copying output, discarding Unknown effects or rewriting started downstream work. These controls grant neither extra tools nor human permission. Parent Turn/instance changes require formal source/control handoff, and inherited references cannot bypass an incompatible build's checkpoint fences.

An exact native child-wait proof may yield model capacity; the original writer readmits computation before it continues. This does not prove that processes exited, file effects settled or Browser/Computer owners released their resources. Pending/unknown effects and real resource owners retain their responsibilities. User Pause, InputRequired, process waiting and child waiting remain distinct contracts. Queries, cache hydration, notifications and an idle Session do not authorize another task or replay a closed Turn.

An exact Returned delegation Effect with verified original tool causation gives admitted children an independent lifecycle. A later parent failure or parent-only stop does not revoke that admission. Explicit delegated-work or whole-Execution stops still fence the relevant children. Pending sources retain their existing running/normally completed parent boundary; Unknown, Rejected and Cancelled effects never substitute for Returned. Accepted cancellation, logical task termination and actual resource cleanup remain separate proofs.

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
`start_process` uses its canonical `wait_ms=0..30000` range with a 1000 ms initial observation by default.
The original owner observes later running-process changes without repeated model polling.
The early-visible component notice reads the same guidance and is derived only from actual
component fields in the exact authorized ToolPlan, preserving visibility budgets and grants.

### User-input handoff and owned-resource waiting

`request_user_input` records the exact question and remaining work in the existing typed
Event/Payload chain and ends the computational Turn normally as `InputRequired`, including when
Session-owned background processes remain alive. Questions are neither answers nor completion
or execution authority. Normal replies use canonical Session context in a new Turn; they do not
resume a native checkpoint. Inputs genuinely admitted before the terminal fence may still be
consumed at that safe boundary. No wait table, question ledger or copied task state is introduced.

Processes belong to their original Session owner and retain their immutable creator Turn, call,
Effect, resource and frozen authority. Later controls require both current and creator grants.
Turn opening and process actions use the same shared comparison for canonical workspace paths.
Ordinary Windows paths and native `\\?\` paths for the same directory preserve authority;
a different directory cannot reuse the original owner.
`wait_ms` bounds one observation and returns the same handle without killing or relaunching it.
`timeout_ms` is an optional actual workload deadline; omission imposes neither a default 5/10-minute
limit nor an idle lease derived from human silence. An explicit deadline remains enforceable.
Turn settlement closes admission and joins dispatched calls; real Session teardown cancels and
reaps the processes. Typed retention in the original cleanup witness distinguishes retained owners from reaped processes.
Only real owner cleanup appends `session/process-owner-released` for the exact original resource set
and generation with reaped receipts. This settles that resource fence without a new ledger or replay
authority. Unknown Effects, binding changes,
workspace relocation and restart retain their owner fences.

The 64-process Session limit covers active handles and handles lacking proven cleanup. Only an
original first-party launch plus its committed terminal owner result and real reap receipt retires
a native handle from that quota. Historical results remain; old handles are never recreated.
Command failure and physical process lifetime are independent. A formally validated original
Exited/Cancelled/TimedOut result may establish launch and cleanup even with `is_error=true`;
final reaping still requires exact ownership and complete cleanup evidence. Original nonzero
exit and failure bytes remain failures. Owner errors, Lost/nonexecuted outcomes, invalid
identities and invalid shapes cannot supply this proof or silently retire uncertain resources.
Before a live Turn commits retention, the original typed `host_process_dispatch.process_owner`
can bind its frozen owner generation and resource digest. Reaping requires the exact original
returned start ToolResult, Effect, creator, Turn Snapshot and native FrozenOutput. Older dispatches
without that binding still require original retention or a terminal ToolResult; current bindings,
PIDs and timestamps cannot supply missing authority. The original Event/Payload chain archives
output before releasing the final handle, without creating another execution or output ledger.
Session deletion and application shutdown drain the shared resource owner even without a Runtime.
After restart, unproven old ownership fences new process operations while ordinary chat remains
available; neither PID adoption nor automatic command restart is permitted.

MCP business uncertainty and native resource closure are separate facts. After the original task
joins and its owner has no active work, the exact canonical MCP cleanup receipt may prove resource
closure while the external business result remains Unknown. Turn and Runtime teardown may consume
that proof; `host_cleanup_proven` never reconciles the business result or proves rollback. Dispatch,
context and checkpoint resume still reject Unknown, and retirement cannot replay work. Missing,
malformed or pending cleanup and another domain's unknown effects cannot borrow this MCP proof.
Native cancellation uses a dedicated settlement entry: one writer transaction checks the original
holder, fence, generation, Turn start and current-generation `host_cleanup_proven`. The exact active
Turn can then settle from reconciliation to cancelled without granting new work or widening generic
event/observation writes. Unknown stays unchanged, and the original journal retries the same terminal
after a failed write. Host activity controls preserve the original Operation's exact identity, including
valid Unicode, within its 1024 UTF-8 byte contract; the control request's 128-byte idempotency-key limit
cannot be applied to that target or used to truncate it.

Human replies have no application timeout. Background output, exit, deadline or external link
expiry never answers a question, reopens model execution or authorizes a fresh launch. The next
user input can continue the task or change topic; continuation verifies the original process or
actual configuration first. External services may expire authorization codes independently.
Pure machine-resource observation may still park an active Turn; it is independent of human
handoff. Native checkpoints retain exact recovery safety without claiming that old handles
survive restart or replaying unknown Effects.

A provider stop ends one response. One stop-resolution decision uses actual plans, current reports
and resource state to select review, live waiting, native suspension or completion. An unfinished
plan alone never proves failure. Exhausted plan, report or end-intent correction retains the work
for human intervention without changing actual command or tool-failure counts. Live resources
stay with their owner; their exit grants no additional correction attempt. Only quiescent resources
can use native suspension. An owner deferred beyond its soft window retains the acknowledged
checkpoint and canonical tail; suspension creates no renewal or invented checkpoint receipt.
Actual faults, failure-stop policy, unknown Effects and patch recovery retain their
original fences and cannot be escaped by a question. Non-effectful input handoffs remain available during locked execution review and after settled command failures, without erasing observations or reopening action tools. A blocked account requires an explicit input handoff rather than a business-dependency native pause.

REST, realtime, history, task progress and domain consumers use the same typed public outcome.
Plan steps describe work progress independently of waiting or suspension; a Turn fault cannot
make every pending item a failed task. A human question closes its Turn as `input_required`; ordinary input remains receivable independently of active execution. IDMM does not treat legitimate waiting as a stalled
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

An explicit question reply references its original source Turn and `question_seq`. Ordinary
messages never implicitly answer the latest question. A late reply retains the original reference
and cannot steer a different active question. Channel delivery records
`channel/input-question-delivered` after platform acknowledgement, preserving the exact original
question and platform/plugin/chat/message identities even after that Turn closes. The SDK publishes
`InputQuestion` immediately after committing the source question, independently of live resource
waiting. Missing proof or an explicit reply reference never selects a newer question. These facts
supply no execution authority, additional model context or question table.

Approved Channel inputs commit trusted `channel_source` with the original accepted body: the actual
transport Session binding, platform/plugin/chat and inbound command identity. Ordinary message JSON
cannot author this source. Inputs for different receivers, or Channel and desktop inputs, cannot
steer each other's active Turn in a shared Session. Only supplements for the same actual
platform/plugin/chat may be consumed together; the Host and Store share this rule and defer the
others to later Turns. Read-only observers address the original input Event ID, publish committed
questions, and deliver complete text and artifacts from the exact canonical terminal. Late stream
fragments cannot substitute for complete delivery. QueueDrain startup and its existing sweep restore
observation after rechecking the original authorization, binding and plugin; they never reroute old
output through a newer binding. `channel/input-delivery-state` records only original input/Turn/transport
delivery start, actual provider ACKs or unknown outcome. It copies no body and adds no queue table.
Confirmed delivery is skipped; an unknown attempt never resends text or artifacts. Question ACK cuts
settle from the original `channel/input-question-delivered` proof. Delivery uncertainty grants no tool
replay or execution authority.

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

Ordinary messages use one `/messages` intake. The Host commits `message/user-accepted` once
and returns its stable message ID, cursor and accepted receipt, independently of execution.
Ordinary delivery always defaults to `queue`: an idle Session prepares and starts an independent
Turn immediately, while a busy Session retains separate FIFO roots. Explicit `prefer_current`
targets the exact observed Turn source and is claimed at a safe boundary without forcibly
interrupting an in-flight model or tool. There is no send-mode setting; `chat.sendKey` only
selects the send/newline gesture. Unclaimed expired targets may become later independent roots;
claimed or delivery-unknown inputs retain their original proof and are never replayed elsewhere.
A single Session scheduler derives pending inputs from the same canonical chain, claims them at
an active safe boundary or starts a later Turn. Consumption references the original input without
copying its body or introducing a queue/recovery ledger. The Store also records new native input
delivery by exact claim/accepted references, restoring typed Runtime content only after validating
Session, Turn, causation and fields. Missing originals fail closed without Runtime-body fallback.
Unclaimed messages never enter context early. A claim alone does not prove native delivery: the
exact claim/input pair must have a committed native receipt, and a closed undelivered claim retains
its visible reason. Exhausting an active Turn's input capacity leaves legal unclaimed inputs for a
later Turn instead of permanently rejecting them or keeping the model sampling. Permanent input
shape errors fail before acceptance; accepted input that cannot currently be prepared retains a
visible typed status without creating a failed Turn or blocking unrelated valid messages.
Cancelling an exact unclaimed input retains its accepted fact and records terminal `CancelledByOwner`.
Preparation retry, later Turn claim and steering cannot revive it. Replaying the original key only
acknowledges that original input; an explicit redo is a new input. Its `cancelled` /
`CANCELLED_BY_OWNER` projection has no edit or retry action and is separate from repairable preparation blocks.
Unknown Effects and technical pauses still fence execution; accepting
text never grants budget increases, cleanup attestations or native resume authority.

Acceptance receipts, input notifications, queue reads and history use one derived `SessionInputView`.
Its placement is `queued`, `steering` or `transcript`; kind, exact Turn source and `order_seq`
express root/steering ownership and the execution boundary. `event_seq` fences late acknowledgements,
notifications and history. The view grants no execution authority and is never written into input payloads.
UI/API contract 46 covers input acceptance, replacement and preparation-stop responses without
changing the Agent data generation, native checkpoints or physical database schema.
Only queued inputs render in the compact composer dock; an idle first input appears directly in the
transcript, and pending steering renders once at its tail before the native receipt places that same
identity between the preceding and following process segments. User message bubbles do not show input-delivery
or execution-state captions; typed metadata still serves exact delivery, queue controls and edit eligibility. Empty docks disappear, one row is
compact, multiple rows initially collapse, and queue-wide operations have a separate scope.

Queue controls use stable keys and versions derived from relevant input/control and Turn-boundary
facts, never token updates. Clear and explicit pause affect queued inputs; they leave starting and
pending-steering transcript inputs alone. Stop retains its accepted-batch cutoff. Editing is local
until save: one writer cancels the old input, accepts the replacement and records references only,
preserving attachments, Skills, effective intent, cross-source order and hold. A claim that wins
the race prevents editing. Canceling the editor leaves the input untouched; an unavailable editing
target keeps its local text visible. Replacement text is stored only in its new accepted payload.
Routing folds and model-boundary reads do not hydrate unrelated queued bodies. After commit,
the existing Host owner takes the wake before observer reads or client notifications. Replaying
consumed or canceled inputs only confirms their exact original receipt.

Process groups follow canonical timeline and steering boundaries. Late fragments merge only
within their own segment, preserving process-before → human steering → process-after order,
public results, questions, approvals and errors. Pagination retains reading anchors and Session-scoped
display expansion choices without holding Runtime objects. Existing `created_at` keyset semantics
remain intact; timeline sequences and input `order_seq` supply explicit execution order.

The renderer keeps unsent drafts and stable payload/idempotency retry identity, without a second
business queue or new-turn/steer/resume routing. Ordinary human handoff shows the question and
normal Send without an end-turn action. Actual generation cancellation remains exact-Turn scoped;
process cancellation belongs to its original resource owner. Closing a Turn preserves Session
context. Forward `/messages` and renderer history derive the same question and outcome from typed
facts with real cursors and complete equal-sequence pagination groups. Public technical pause
notices remain exact-operation observations, never cleanup or recovery authority.

A valid, current blocked account never permits completion. Plain text in place of the required input handoff uses the existing bounded `ModelResponseRejected` and `ProtocolRecovery` budget. `tool_choice` is a request; only a committed typed question or actual accepted user input establishes that handoff.

Ordinary `report_completion.summary` is the complete final answer, without a separate 2048-character
abstract limit. Its summary, criteria, references and JSON escaping share the existing 48KiB serialized
UTF-8 argument budget; character counts cannot stand in for encoded bytes. Exceeding that total rejects
the complete report with the actual budget, without truncating text, closing the plan or authorizing
reexecution. Evidence, cumulative failure accounting and the four-correction guard remain unchanged.
Strict historical publication keeps its independently bounded 2048-character `short_summary`: that is
only an introduction to exact owner-selected results, whose existing reference and publication budgets
still apply. Explicit Agent continuation retains the complete historical answer under the existing
80KiB handoff envelope, without another short-summary cap. All content uses the original Event/Payload
chain; the 64KiB inline, 1MiB single-Payload and native checkpoint storage bounds remain unchanged.

Queued inputs freeze their history boundary at their actual Turn claim, rather than their earlier
acceptance. They therefore retain the preceding Turn's complete output committed while queued.
Closed Turns and domain DATA also merge at the actual claim sequence; early acceptance cannot
stand in for execution order. The current input appears once, and later unclaimed messages never
enter that history window.

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
content remain exact. Arbitrary release, capability-name or digest exceptions do not grant compatibility.

Migration 010 preserves the same Agent data generation, baseline, projection and all Agent facts while
adding two OAuth configuration tables. The formal schema manifest proves that exact predecessor-to-current
relationship for bundled contract evolution and Session extension derivation. Unknown manifests, reverse
transitions and different generations remain rejected. Old Revisions, Snapshots and events stay immutable;
official Agent launch failures retain their backend error code and diagnostic reason for display.

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
| `companion.default` | One Companion Agent with persona, shared memory, channels, schedules, ordinary work tools and Robot Actions. Physical tools require exact authorized device resources. |
| `customer-service.default` | Customer notes, handoff, knowledge retrieval, channel replies and sends, files, processes, artifacts, research and schedules; no default global knowledge-registry, cross-Session or terminal management. |
| `creative-studio.default` | Media, Canvas, assets, templates, Office, files, processes, artifacts, project memory, research, browser, collaboration and schedules; retains its three creative Package Skills. |

General and Companion grant only asset read/write in `creative.workshop`; a missing Canvas cannot hide those Actions. Native Office tools edit or preview asset-library documents, sheets and slides. Local-file workflows continue through authorized files/process tools. Global Skills and MCP selections retain their exact compilation path and do not substitute for native Action grants.

Frozen `CapabilityActionDescriptor.required_resource_operations` has explicit optional semantics. `None` or an omitted field retains the complete Module resource gate; sibling Actions cannot imply that an unspecified Action is resource-free. `Some(empty)` explicitly consumes no resource slots, while `Some(map)` requires a binding of each named kind with every declared operation. Model-visible tools and Kernel admission use the same frozen check. Context/Event consumers keep the Module resource check, and Role Providers keep their exact implementation slots. Missing optional targets disable only dependent Actions; filtering never bypasses owner, Snapshot, active-capability or invocation authority.

Ordinary creation and product entry supply deterministic host resources under the target's frozen grants: Workspace, Process Session, project memory, asset library, Scheduler, and applicable Terminal, Browser and Computer resources. Product identity uses the actual Companion, Customer or Canvas target ID; Workspace and Process share one root. Published Browser/Computer Role contracts may be frozen while their optional live environment remains unbound. Knowledge bases, channels, SSH hosts and robots require actual selections, without invented connections or credentials. Owner mismatches, storage I/O or corruption, contract/Provider mismatches, native checkpoint safety and unknown effects retain their fail-closed boundaries.

Computer invocations retain the canonical Session/Turn, native lease/claim, exact unsettled call, Snapshot, Action, Provider and resource checks. Observation creates no Effect; input and launch settle through the existing Effect chain. One App-owned Computer owner fairly serializes Sessions. Host-generated worker generations and driver-owned native sessions, snapshots and `element_token` values are independent of canonical event sequences; the Store issues no observation credentials and history grants no execution authority. Each model-attempt presentation receipt verifies only that attempt's projected schema. OS prerequisites follow frozen `system_permissions` and the prepared arguments before dispatch; confirmed missing permissions mean zero execution, while unknown effects are never replayed. See [Computer Use](computer-use.md) for lifecycle, model input tiers and platform boundaries.

Saved official customizations are not overwritten by release seeds. The owner-scoped `GET /api/agent-presets/{preset_id}/recommended-capabilities` returns a read-only merged draft and missing Module/Action/Package Skill additions. Persona, instructions, model, Runtime settings, existing extensions and pinned Skill versions remain authored choices. The UI shows additions before saving through the existing SaveRevision `expected_current_revision` CAS and does not replace unsaved edits. Saving leaves existing Sessions unchanged. A Companion's explicit Apply latest command uses the canonical Agent switch at an idle, settled boundary, preparing the current release definition for an explicit Template selection or the saved configuration for a Preset selection under the Session operation and binding-version fences. Direct Session preview/apply additionally requires the exact previewed target Snapshot. Both retain the Session and historical artifacts without a separate upgrade or binding ledger.

`knowledge.management` uses the existing domain service for list, inspect, managed creation, metadata/URL-source updates and deletion. Partial creation returns the committed base ID and actual document/source status, so retries repair that base. Mounting uses the existing knowledge selector at an idle boundary; a tool cannot rewrite the running Turn's frozen binding. `session.management` queries and creates canonical Sessions, sends input to another idle ordinary local conversation, cancels an exact observed Turn, and manages real PTYs and linked AgentExecutions. Input uses canonical idempotency and binding/context fences. Terminal input/relaunch/stop check owner, row version and PTY epoch; Execution mutations require the active lead/attempt relation and exact versions. Target results and terminal output are data, not borrowed authority or permission to replay unknown effects. Effects settle through the existing canonical chain.

Companion persona and historical memory never advertise retired `nomi_*` tools or unverified full capabilities. Current Runtime tool descriptions and real receipts determine names, inputs, target scope and completion. Authorized catalog search finds deferred tools; their absence from the initial schema display does not prove a missing capability.
Canonical Runtime persona/instructions come from this admission's exact frozen Revision
payload. Product persona Context and current facts are assembled separately; old extra
memory snapshots are not restored as static model instructions.

One Companion identity selects one primary Agent, default chat model and Skill/MCP setup. Desktop, quick-window, approved channels and physical devices use independent canonical Sessions. Reconnection preserves the device Session; navigation does not redirect input. Product Agent configuration uses only `companion/<companion_id>`; typed physical origin remains a source and authorization boundary.

Authorization, schema display, context projection and device assembly are separate. Compact physical interaction does not depend on a fixed narrow capability set. Adding research, files or knowledge retains ordinary authorized tools and the same interaction policy. Deferred descriptions do not change authority. Unbound or offline devices do not block unrelated work; large schemas, Skill bodies and historical results expand on demand.

Ordinary queries and operations execute directly in the receiving Session. AgentExecution remains available for durable, isolated or parallel work with canonical task references and user constraints. It is selected for actual task benefit, not an entry-point capability gap. The source parent Turn reads exact child results, integrates and verifies them, then delivers through its ordinary Runtime. No independent background report or hidden result-model Turn replaces this work. Direct user-created Executions without a tool source retain their summary projection. Normal progress and recovery consume the same original facts.

Explicit Session reasoning settings remain. A transient responsive preference only uses exact documented provider/model/protocol support and never rewrites saved models or frozen grants.

The optional `finish_without_reply` Runtime control declares that the entire physical interaction needs no public answer.
It can follow the final motion/display batch, with real execution still serialized. Host settlement, cancellation and the
atomic terminal-input fence must pass before the typed `silent_completion` observation is committed. An exact completed
Turn with that intent and its empty closing seal derives `AgentTurnOutcome::Silent`; old empty answers remain unavailable.
This is reply intent, never evidence that physical movement or delegated work finished. UI history retains tool activity
without a manufactured answer, while Robot receives only `Done` and does not start TTS. UI/API contract version 40 requires
matching frontend and backend builds. Other Agent empty-answer and delivery guarantees stay unchanged.

Robot ASR runs as an owned task, with at most one recognition in flight and two queued recordings per connection.
The device reader continues handling MCP replies, abort, disconnect and ping. Abort clears unsubmitted audio
and joins recognition cleanup. Replaced connections and host shutdown cannot submit late transcripts as new
Companion inputs. Independent recognition scheduling retains exact source and generation fences; committed public prefixes can be delivered before the whole work Turn ends.

The conversation list's `archived` field is a canonical metadata annotation. It preserves
history, working context, active tasks and ingress routes. It is separate from automatic
Companion window summaries, their fenced context reset, explicit context clear and deletion.
Listing and ensure read the current annotation from canonical metadata rather than an old
navigation-table field; archiving does not redirect incoming device or quick-window input.

The App's compilation surface remains desktop regardless of optional Browser/Computer adapters. Deferring physical availability in the resource ceiling requires an exact pinned bundled public Provider with one member, one resource kind, complete Action operation contracts, and no private implementation, dependencies, Context or Events. Unsupported physical members enter only the current compiled object's non-executable set, never Snapshot or checkpoint data; binding and invocation still reject them. Other Providers, incomplete contracts and direct non-Agent operations retain strict admission.

The authorized catalog is not truncated at 128 Modules or Actions. With ToolSearch, initial schema presentation projects within 24 KiB and 107 tools, reserving 16 Runtime control slots and five latest search matches while preserving every binding and schema in the original ToolPlan. Later searches protect the latest set of up to five matches, fill remaining space with older displayed tools, and leave omitted tools searchable. Explicit Host preactivation uses the final authorized frozen names and atomically presents the entire requested set; it does not split them into five-match searches that can evict earlier members. Both latest search results and explicit Host sets may exceed the soft 24 KiB target. The hard limits remain 112 authorized definitions, 128 visible tools including Runtime controls, and the whole-request context budget; an oversized set is rejected without partially changing activation. Canonical recovery rebuilds presentation from its original facts and applies the corresponding set rules. Without ToolSearch, only the actual assembled visible tool count is limited. This window is presentation state, not a second authority set or persistent ledger.

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
Ordinary human handoff closes with `InputRequired`, settling any retained Browser work through the terminal lifecycle
while preserving Session background processes. A real parked machine-resource or intervention wait may yield Browser
within the same Turn: the Store verifies the exact owner, active Operation, committed `waiting` outcome and absence of
pending/unknown Effects before and after settlement. Only Browser ownership is released; the next Browser action needs
a fresh lazy admission and observation under the frozen grants. Normal per-call release instead preserves usable
observations within the same Turn; native target, document and observation checks reject navigation, closed tabs and
replaced runtime generations. Terminal cleanup still drains retained work and retries failed releases. User APIs use
the actual coordinator gate, without deriving ownership from Session running or paused, model prose or configured
Browser permissions. Chat, file work, attached Chrome and isolated background rendering leave the managed browser
available whenever it has no active browser owner.
User profiles use owner/Session identity independent of grant definitions: Windows uses
`browser-v4/agent-sessions/<hash>/`; macOS 14+ uses a separate persistent
WKWebsiteDataStore with a stable derived identifier and does not read former directory profiles. Session deletion includes this browser and its
profile even when no Agent Browser binding exists; there is no second active Agent authority ledger.
See the [browser architecture](browser-platform.md).

## Global extensions and composer selection

Skills and MCP are shared installation capabilities without a preset master gate. Creation or an explicit
idle update captures the global Skill inventory and compiles selected MCP tools into the same
Session-only Revision/Snapshot. All MCP sources require an explicit Agent or Session choice; enabling
or testing an installed source never selects it by default. Omitting Session selection keeps default
Skills and the Agent's saved MCP grants without adding other installed sources.
Variants use the existing Agent Store, with no second binding or
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

MCP business execution has no default total or silence deadline; setup and
cancellation/cleanup remain bounded separately. Cancellation fences new
admission and targets the exact original request while the original TaskGroup
retains execution and cleanup. The native lease remains renewable until owner
settlement or an explicit fence, rather than treating a caller ending its wait
as producer exit. Results, dispatch facts and local/remote cleanup observations
remain on the original Event/Payload/Effect chain. An observed result survives
unproven cleanup. Only a proved pre-effect rejection can settle as rejected;
unknown effects still forbid replay. Memory owns mutual exclusion, not a second
permanent unknown ledger; reconciliation must retain exact-instance cleanup proof.

## Consumers and deletion

UI consumes current stream and Message projections directly. It has no marker-dependent local terminal-processing state machine or prose-based reclassification of old errors. Channel binds through its current owner; orphan rows without an authority binding produce a conflict instead of being automatically rebound.

### Canonical process timeline

Live and historical display share a read-only timeline derived from canonical content-part and typed Runtime tool events. Existing content writers, seals and `agent_messages` remain the content index. Narration and readable reasoning have independent body lifecycles: reasoning deltas, completion and signatures never split one text source; within the same model step and activity interval, one reasoning source remains continuous across narration deltas, narration completion and narration source changes. Channel switching, transport packetization, character counts and punctuation do not close a reasoning body. Runtime buffering and history reads share one reasoning-boundary classifier over existing event kinds. Tool proposals/calls, model-step changes, accepted steering, withdrawal and reviewed delivery retain explicit boundaries, without merging an entire step across tools. Blocks reference exact UTF-16 half-open source ranges or the original tool call, without another body, phase table or checkpoint. The first canonical content event identifies a block; the accepted root or preceding real narration boundary anchors its process group. A new text source closes only the corresponding narration body. Late tool results update their original group.

The Store returns source content and timeline references in one read snapshot with `through_seq`, preserving the raw pagination cursor. Partial Turns retain their real group anchors without loading the entire Turn. Live text frames atomically carry their source's canonical timeline; a later, best-effort notification cannot establish a new text boundary. If the exact display snapshot is temporarily unavailable, a Turn-local queue retains unpublished text within the existing stream budget and releases it with the recovered boundary in order, without rewriting canonical content or converting a display failure into an execution failure. Ordinary uncommitted tails reuse the same source's confirmed range. The separate `canonical_timeline` notification only updates existing rows in its exact Turn; it cannot raise busy state or create another terminal. Display refresh follows actual buffered commits, while uncommitted token tails remain ephemeral. Recovery preserves the existing bounded content policy and voice withdrawal boundaries. Both pause notifications and authoritative pause reconciliation invalidate canonical history to recover any deferred text; this content refresh neither announces completion nor releases the paused Turn or its input queue.

Optional `ChatOutputTextSource` metadata preserves validated provider text item, output position and content-block identities through the existing Runtime event and `message/content-part.source`. Distinct Responses and Anthropic text blocks remain distinct. Protocols without native block identities use the existing canonical model-step message source. Existing current-generation events without this metadata derive continuous narration from their original message identities and real tool/step boundaries; reads do not backfill identities, rewrite history or create another content table. Runtime content assembly follows the same source contract: ordinary unsigned reasoning does not split its text, while signed, opaque, provider-reasoning and tool blocks remain native barriers with their original ordering and continuation state.

Each text source and reasoning activity commits its first real fragment before live publication so the existing UTF-16 reference identifies its active tail. Reasoning does not reopen narration. Later fragments retain the bounded 16KiB buffering policy, and a batch cannot mix text sources. A checkpoint flush does not reopen the source, and late tool settlement does not split it. Completion reports still publish atomically after their original review and settlement gates, with a separate reading boundary.

A tool boundary belongs to the first proposal/invocation for its original Turn, model step and `call_id`. Later argument fragments, completion confirmations, starts and late results reuse that invocation instead of repeatedly closing reasoning. A proposal without a visible invocation retains only its original audit identity, without inventing execution or results.

OpenAI-compatible reasoning aliases select the first nonempty string in their protocol's existing priority order. An empty `reasoning_content` cannot shadow valid `reasoning`; generic reasoning aliases follow the same rule. Only empty strings are skipped: whitespace, punctuation and short words remain exact, and mixed-frame narration/reasoning publication order stays unchanged. Correct segmentation never relies on reordering fields, completing sentences or filtering short fragments. Existing current-generation history derives corrected ranges from its original facts without rewriting events, bodies or native checkpoints.

Process narration and public answers share `ConversationProse` and a Session-scoped ephemeral display cursor. Messages retain their original zero-delay ingress batches and complete received text; copying, result membership, cancellation and Runtime busy state never wait for animation. Live ingress explicitly admits sources for frame-paced presentation, sharing progress across exact canonical source/ranges; history reads do not grant live eligibility or replay a process-to-answer handoff. Each narration block uses one Markdown tree, reused within its presented source revision, revealing only arrived characters and complete syntax ranges. Code blocks and display math keep stable widgets containing their actual received content; other Markdown consumers keep their default renderer. Cursors are neither persisted nor another appended body. Replacement, withdrawal, cancellation, pause, Session changes and reduced-motion settings retain their immediate boundaries.

ToolCall messages retain bounded capability/action, name, target and typed-result summaries. Commands and general tools offer on-demand detail. File activities default to summaries and expand only into read, edit, delete or rename paths and verified change counts, never file bodies, raw arguments or patches. Live and history use the same bounded `file_summary` derived from exact native file identities, typed requests and committed results. Only adjacent confirmed activities in the same Session, Turn and process group combine; paged reads deduplicate by their original paths. Failures, partial completion, Runtime holds and unconfirmed results never infer change counts. History inspection never reads the current disk. The bounded first shell line or literal program/arguments identifies commands; full scripts remain in canonical storage. Unknown MCP/plugin names use general detail, without inferring file effects or execution status. Native outcomes, permission prompts, bounded-search notices, knowledge writebacks and verified media retain their host-authored semantics.

A native `report_completion` is hidden as bookkeeping only when completed with a valid original result sequence, no error or Runtime hold, and a confirmed observation. Explicit capability/action identities, same-named MCP tools and failed or unconfirmed proposals remain visible. Its raw report arguments and result are not repeated in process detail; errors remain inspectable. This is not text deduplication, receipt acceptance does not prove task success, and public delivery, completion review and Turn outcome membership remain unchanged.

`GET /api/agent-sessions/{session}/tool-details` reads one exact accepted Turn and invocation. One Store transaction verifies owner, causation and shared Payload integrity. A byte-page cursor pins the content digest, result and committed upper bound; later results cannot enter an older page. Committed empty output, output not yet submitted, unconfirmed terminal observations and corrupt storage are distinct. Late original host settlement after cancellation remains readable, without borrowing a later Turn or replaying a process archive. Native output retention, loss and decoding metadata remain visible; `eof` refers only to that committed section. Inspection acquires no Runtime or execution authority and never runs, resumes or retries a tool.

Detail DTOs and bounded request caches are separate from TMessage. A confirmed call/result identity is separate from its read snapshot: an opened inspector pins its first upper bound, preserving loaded pages and reading position as unrelated Session events arrive. Result changes and reconnect invalidation select a new snapshot. Ordinary, companion, read-only task and creative process views share their inspection semantics; companion bubbles remain summaries. Opening detail neither changes exports nor causes exports to fetch raw content. The UI/API display contract includes the read-only display endpoints and file activity summaries; display schemas are separate from persisted execution schemas and do not trigger an Agent dataset rebuild.

`ToolCompleted.runtime_hold` records only a bounded Runtime reason why one proposal did not reach its owner port. `ToolStarted` remains write-ahead admission intent. Discovery, held proposals, owner failures, cancellation and unknown Effects keep distinct semantics. Missing historical reasons remain unclassified rather than guessed from English output. Host settlement remains readable when cancellation prevents a Runtime completion. Display metadata never changes authority, work accounting, delivery review or replay decisions. New reports use the v5 public format without automatically appended aggregate failure counts. Work accounting and delivery gates retain their original values; saved current-generation older deliveries keep exact text validation without rewriting history.

Shared conversations retain canonical segment identities while rendering reasoning, tools and delegation records as flat rows. Narration keeps its original reading boundaries and `AgentTurnOutcome` owns public-result membership. Muted thinking, tool summaries, delegation and task names share the body font size rather than a separate smaller process size. Rows and expanded details share one left edge, without nested indentation, leading rails, tabs or per-tool elapsed time. Delegation uses compact action-and-summary rows with flat task lists on demand; [Execution presentation](agent-execution.zh.md) owns source grouping. The overall work clock remains independent. Terminal default collapse applies only before a user makes a disclosure choice; increments, completion and refresh preserve that choice and reading position. Unopened detail does not prefetch tool bodies. Closing group membership does not prove parallel tools settled. Permission actions, deliverables and user questions retain their existing contracts.

Only outer group titles pulse while processing, independently of expansion; detail rows never pulse. Within an open Turn, groups with running members and the trailing group before any later narration or group retain title activity, including model-response and argument-generation waits. Ordinary model status rows do not end that continuation. This derives presentation activity without changing tool outcomes, canonical segment states or expansion defaults. Turn settlement stops the pulse immediately, and reduced-motion preferences disable it.

Error diagnostics capture the Agent, model and workspace admitted for the exact Turn and retain them with the original detail in the existing canonical terminal payload. The backend emits typed `taskIncompleteReason` for incomplete tasks; the renderer never guesses the cause from diagnostic prose. Live errors and historical projections consume the same terminal information. Missing historical fields remain unrecorded rather than borrowing the Session's current selection. User-facing reasons and recommended actions are separate from collapsed technical details.

One `ModelFailureDiagnostic` carries model-request refinements and evidence from invocation through the Broker, typed failure and existing terminal into `providerDiagnostic`; classification is never reconstructed from English messages, UI projections or unknown response bodies. Explicit upstream machine codes take precedence over broad status. When HTTP 401, 403, 404 or 429 alone cannot prove credential expiration, model access, an incorrect API path or depleted balance, the explanation retains that uncertainty. Diagnostics capture the sanitized actual endpoint, provider and model, protocol/auth method, HTTP status, upstream code/type/parameter, request ID and retry hint; arbitrary response prose, credentials and authentication-header values are excluded. Safe local transport causes remain available in technical details, without changing retry, authorization or settlement rules. Failed-attempt identity after failover is shown separately from the Session-selected model; account links require trusted configuration matching the recorded provider identity.

Failures, current pauses and rejected requests share one error presentation. Model or provider failures automatically unlock the composer after the existing owner proves cleanup and native resource release and commits canonical `turn/failed`; the user does not need to end the Turn again. The same owner may settle an already-saved current-generation typed model failure pause during startup, Session reads or the next admission. Its transaction rechecks the exact pause revision, execution fence, Snapshot, current-generation cleanup receipt and absence of pending/unknown effects. It never runs a model, replays a tool, or changes failure into success or cancellation. Unproven cleanup, unknown effects, manual pauses and other recovery pauses retain their native fences and explicit recovery authority. Genuine pause notes derive only from verified current state and never enter Message history or invent failed terminals. Rejected requests remain transient within their originating Session, and an admitted request's canonical error replaces its HTTP fallback.

Cron, Companion, Requirements, AutoWork, IDMM and AgentExecution retain their own business configuration and supervision state. Session input, admission, cancellation and completion use canonical Turn receipts.

Cron's private dispatcher records its server-validated job/run/Session reference as typed
`cron_runtime_overlay` on the original `message/user-accepted` input. Ordinary message requests
cannot supply it; text, colliding operation keys, mutable job settings and message projections do
not establish source ownership. Existing inputs are not rewritten. This changes neither
AgentInputSource, Robot delivery nor model grants. Resource retirement for a new per-run Session
retains its original creation and finalized receipt even after legitimate job/history deletion,
without adding a persistent ledger alongside the original Session and domain facts.

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

The Companion selects `companion.default` or a personal Agent with the identity and memory core. The complete capability catalog remains editable. Resource resolution uses the exact target companion; another companion's identity or memory cannot be inherited. The owner-scoped canonical opening, typed origin and frozen binding establish product and device ownership. Mounting a Companion resource in an ordinary Session does not create product membership. Independent entry-point Sessions retain their own input routes.

Voice from a paired robot enters its dedicated Robot Session. Admission holds the
current connection fence and verifies the exact canonical Companion, role and
device. The authenticated host records typed input provenance; ordinary requests
cannot invent it through an origin string. Camera permission and robot vision are
independent. Voice input does not alter frozen grants; tools retain resource,
Turn, connection and device-permission checks.

The original parent Turn reads canonical child-result references and integrates them; public output and physical speech use its ordinary Runtime delivery path. A child terminal, notification or temporarily idle user Session cannot create another model Turn. Questions still reference their committed request and verified presentation. Physical audio freezes its presented question identity at the recording/ASR boundary; slow recognition cannot acquire a later question or another connection's authority. Replayed sources do not roll back the question cut.

Direct device requests use the ordinary typed `request_user_input` path. Answers retain the original question identity and accepted human root; model-authored answer parameters cannot substitute for user input. Multiple pending questions, successor questions and click-required approvals do not gain permission through speech. Closed parent inputs are read through the exact read-only accepted-input API without borrowing active native admission.

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

Historical Execution dependencies and known-receipt verification use the Store's read-only
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

Empty Attempt-created Session retirement uses a dedicated Store admission within the existing deletion lifecycle. Under the same SQLite writer as input and link admission, it proves the exact owner, original Attempt and opening key, a durable terminal or superseded domain disposition, and only the original opening, generation-zero activation and optional ready facts with their exact identities and causes. Any historical link, input, Turn, Effect or later event preserves the Session. A still-owned Queued reservation waits for its domain disposition instead of being deleted because an old caller lost its lease. Once fenced, the original lifecycle owner retires the exact warm Runtime/process resources, managed workspace and IDMM state before the final purge. Busy handling and caller loss retain cleanup responsibility; restart derives pre-fence candidates from the original Attempt/opening and resumes fenced work through `deleting`, without another ledger.

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

### Bounded process launch and owner input

The current process launch contract is `workspace.process/start`. It returns a terminal observation within a bounded initial window, or the same owner's running handle, retained output and cursor. The default observation window is 1000 ms; returning does not terminate the process, renew its hard deadline or prove command success. Synchronous exec execution is retired; historical events and immutable snapshots remain read-only. Existing Sessions evolve exact bundled exec grants to the equivalent launch grant at the idle CAS boundary. New Sessions opened from an older Agent reuse the same proof to derive a hidden execution configuration while preserving its authored Revision/Snapshot. Neither path grants process controls or translates old checkpoints.

Owner output observations do not consume pipes or renew leases. Waiting to understand new output wakes on a real cursor advance or terminal; after asking the user, only accepted input or the original resource terminal wakes model sampling. Spinner output is not model progress. Settled command failures and report-only phases preserve non-effectful input handoffs without reopening effect admission or erasing failures. Blocked completion accounts require an explicit handoff rather than a business-dependency native pause or false task delivery.
