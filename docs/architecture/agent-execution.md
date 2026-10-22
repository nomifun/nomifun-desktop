# Agent Execution and collaboration

AgentExecution is the domain aggregate for one goal. AgentSession remains the sole authority for Sessions, Turns, events, content, effects and cancellation. Execution composes the current Runtime; it does not implement another model/tool loop, transcript or completion contract. See [Agent Session](agent-session.md) and the [Chinese execution architecture](agent-execution.zh.md).

## Ownership

`AgentExecutionEngine` is the public facade. The Planner produces a DAG, the Router assigns Participants, the private Scheduler manages readiness, leases and concurrency, and AttemptRunner executes each Attempt through the existing Session owner. Other domains use facade commands or narrow read guards rather than updating execution state directly.

| Record | Responsibility |
| --- | --- |
| AgentExecution | Goal, policies, version and lifecycle |
| Participant | Immutable Agent/model snapshot available to this execution |
| Step / Dependency | Work unit and blocker-to-blocked DAG |
| Attempt | Actual dispatch configuration, operation identity and result |
| ConversationExecutionLink | Explicit Session-to-Execution/Step/Attempt relation |
| ExecutionEvent | Durable state, decision and audit facts |
| AgentExecutionTemplate | Reusable creation input, captured once when instantiated |

An active Execution never rereads a mutable template. Template edits or deletion do not change an existing snapshot. Delegation within an Attempt appends Steps atomically to the same aggregate, reusing its Participants, policies, Scheduler and lifecycle. It does not create nested execution aggregates. Domain events reference Session facts without replacing model, tool or Effect evidence.

## Policies and concurrency

The top-level policies remain independent:

| Policy | Meaning |
| --- | --- |
| `delegation_policy` | Disabled, automatic or prefer parallel |
| `decision_policy` | Automatic decisions or ask the user |
| `adaptation_policy` | Fixed plan or bounded retry, reassignment and replanning |

A free-form Step role is descriptive. The typed tool policy only narrows the inherited Session authority; a role cannot grant tools. Each Attempt records the effective configuration. Model pools and the current lead/template selection are validated together; a bounded pool must contain its lead.

The shared bounds are defined in [agent_execution.rs](../../crates/backend/nomifun-common/src/agent_execution.rs): 16 models, 64 current Participants, 128 current Steps, parallelism up to 64, and delegation depth up to four. Historical revisions do not count as current work and cannot be deleted to satisfy limits. Templates use the same executable bounds.

Ready children may run concurrently under the user's `max_parallel`, Participant limits, actual dependencies and native reservations. `Full` or `ReadShell` no longer makes a complete Step an exclusive workspace lock. The tool policy remains an authority ceiling; parallelism cannot silently replace Full with ReadOnly and remove selected capabilities. Exact file, process, Browser, Computer and Effect owners continue to enforce their own operation and settlement rules.

One Session still admits one model Turn at a time. Independent child Sessions can execute together. Model capacity is separate from real resource ownership: an exact InputRequired terminal or native child-wait proof can yield a model slot, but display status, checkpoint presence, idle state and cache eviction do not prove resource release. Computation must pass the original writer's capacity fence before resuming.

## Children and the original parent Turn

`agent/delegate` with `strategy=parallel` materializes a bounded `tasks` array as ordinary independent Agent roots. `strategy=planned` asks the current Planner to derive the goal's real dependency DAG. The retired `synthesize` switch is rejected. Explicit synthesis Steps and templates remain ordinary supported DAG work; a hidden automatically appended child does not replace parent integration.

The acceptance receipt retains `execution_id` and any committed `step_ids`. A new execution can still be planning, so its Step IDs may appear through subsequent status reads. Acceptance proves neither startup nor completion. Explicit collaboration intent preserves the parent's complete frozen ToolPlan and does not force every model step to delegate. Already accepted work must not be resubmitted under fresh call IDs. Recovery reuses the original Effect and domain idempotency facts. Later work from the same source Turn and delegation within an Attempt remain in the original Execution, with ownership from the exact Effect and `PlanChanged` facts rather than names, directories or inferred DAG ancestry.

The parent can continue independent work before joining children:

- `agent_status` reads work belonging to the exact source Session and Turn, optionally selecting precise Steps. Result bodies and artifacts come from the existing canonical output reader with Execution, Step, Attempt, child Session, Turn and terminal-event references. UI summaries and child prose are not integration evidence.
- `agent_message` sends an idempotent message to an owned Step through the existing Step and Session command path. It cannot address unrelated work or widen permissions.
- `agent_wait` waits for all selected child work or a new actionable state. It records typed `AwaitingChildren` and paired `ChildrenWaitResolved` observations within the original Turn.

The wait checkpoint retains the original operation, model steps, budget accounting, plan, settled tool observations and wait selection. Recovery reattaches the wait instead of spawning children again or renewing model budgets. A wake is a data observation, not a fabricated human input or completion proof; the parent reads canonical status and integrates the results. Ordinary queued inputs retain FIFO, while explicit steering joins at the original safe boundary. Stop fences reject late wakes without selecting a successor Turn.

New real questions and a scheduler pause carry their original question/control event identity and wake the parent once. Unchanged attention state does not repeatedly sample the model. If planning formally fails or is cancelled before any Step exists, status returns the original aggregate terminal receipt: an empty set is neither success nor a reason to wait forever. Result cursors cover all owned terminal/question identities independently of which result bodies this status call selects. Controller-domain outcomes and ordinary Agent canonical Turn output retain their separate source readers.

Result bodies use UTF-8 byte pages of at most 32 KiB. `result_page` pins the immutable Attempt/native terminal source, the complete body's SHA-256 and length, and exact offsets. A response with no remaining body budget returns an explicit unread first-page reference; it never reports truncated text as a complete result. The parent consumes a terminal child only after contiguous coverage from zero through the complete original body. Reading only its final page is insufficient. The native checkpoint retains bounded source/digest/coverage references, with bodies remaining in their original canonical events. Full ownership and wake cursors do not change when selecting another body page.

An exact child result also reports whether that Turn's workspace effects or native process commands may have changed the workspace. Partial or unknown effects invalidate previous observations; only the process owner's exact typed pre-I/O rejection can exclude an unexecuted command. Each new immutable source advances the parent's work epoch and invalidates previous completion evidence and scoped-instruction observations once. Repeated reads of the same result do not repeatedly invalidate them. Tool policy and unrelated Execution event growth are not mutation evidence.

`workspace_change_sources` covers every original Attempt and its exact initial/question/decision Turns, independently of the current result body. A newer zero-start cancelled Attempt cannot erase an earlier writer. A business-terminal Agent whose original native Turn is still running remains `delivery_pending` attention and is excluded from consumable terminal IDs. An exact closed native receipt or durable domain admission fence plus proven absence of that exact Turn resolves delivery; `started_at`, a guessed latest Turn and display cancellation cannot prove zero startup. Native result closure remains distinct from heavy resource cleanup.

Child waiting is distinct from user Pause, InputRequired and process waiting. Yielding model capacity does not close real resources or erase pending/unknown effects. Queries, notifications, cache hydration and an idle Session do not authorize new work or replay a closed Turn.

Each ChildAttempt creates its own Session and Runtime from the exact immutable Snapshot, model and typed resource bindings, then applies its explicit Attempt ceiling. It does not inherit the parent's temporary schema window. Attachments and explicit Skill selections from the original accepted input and steering actually applied before delegation pass through exact source/Effect/Turn/owner checks into ordinary child input preparation. Skills must remain in the child Snapshot and media must satisfy its real model/resource constraints. The child retains its own Step spec; original user constraints are source data, not another copied transcript or a permission grant.

An explicit followup reuses the original child's Session and verified frozen binding while preserving its prior canonical history. Its `message_source.command_event_id` points to the actual native `ToolCallCompleted` proposal. Delegation ownership stays with the original source; input preparation reads the followup sender's exact accepted Turn and only steering applied before that command. It neither reinjects the first delegation's selections nor chooses the latest input. Messages to running children resolve these same selections through the original StepChanged event, target Attempt and command. Initial dispatch, running guidance and completed-child continuation all retain selected files and Skills through canonical input preparation. A direct UI followup without model-source input metadata retains its own input and does not borrow old selections. Reader and writer share the immutable command, target, content digest and idempotency binding proof; current native-epoch checks govern new command admission separately.

After explicit steering joins the parent Turn, question answers and recording handoffs read the original input of the last typed SteeringInputs actually applied before the current tool's cause. They use the original Turn input only when no such steering exists. Queued inputs and inputs arriving after the command cannot authorize that answer, question presentation or recording handoff.

Robot child-question notices and actual playback share a transaction-scoped source-authority check. New notices cite the original delegation's real Returned Effect terminal event, allowing the coordinating parent Turn to remain active. A running source must still be the exact active Turn and pass domain admission; existing canonical notices from completed source Turns retain their genuine terminal proof. The Store's original accepted-input and Effect readers supply the sole fact validation. Current binding, clear floor, device, connection, exact pending Step/Attempt and playback writer lease continue to govern delivery; cancellation or an expired presentation cannot authorize replay.

## Results and public delivery

Normal settlement, restart recovery and explicit adoption use [canonical_output.rs](../../crates/backend/nomifun-agent-execution/src/canonical_output.rs). The reader resolves the exact Turn's event window and terminal output in one transaction. Artifacts must come from settled file or publication receipts with verified root, path, length and digest. Directory scans, UI messages and model claims cannot establish them. Later changes to the same path are interpreted in event order.

A Step spec is task input. The sole Runtime owns typed requirements, completion and delivery. The Scheduler does not infer a second acceptance gate from natural-language file names, formats or counts. Exact canonical results are available independently of abbreviated UI summaries.

For source-authored delegation, the parent Turn reads child results, integrates and verifies them, and publishes its ordinary Runtime answer. No independent background-report writer publishes a child's output as the parent's final answer, and no hidden user message starts a second result-model pipeline. Robot and Channel delivery retain the original input, binding and connection authority. Direct user-created Executions without a tool source retain their idempotent public-summary projection. AutoWork's root still executes in its original lead Session, avoiding duplicate public output.

A later parent failure or parent-only stop does not revoke a Returned delegation with verified original causation. Children remain subject to their Step, delegated-work, whole-Execution and native admission fences. Real human-input handoffs continue task authority and result references only through verified, decreasing `reply_to` ancestry with the exact terminal question, current binding and clear floor. An unrelated new Turn gains no general authority over old collaboration.

Deleting an already consumed Execution does not turn its old consumption index into a live prerequisite for a later human reply. Pending deleted work still requires its original source proof; the parent can observe only the exact original `Deleted` event and persisted terminal status as user revocation with outputs unavailable. That receipt resolves the pending handoff without declaring child success, restoring ordinary reads, sending guidance, or reopening scheduling. Missing, foreign or damaged deletion evidence remains an error.

`request_user_input` normally closes the child Turn with InputRequired; that does not complete its Step. Questions retain their exact DecisionRequested and presentation identities. Background process output, exit or timeout cannot replace a human answer or reopen model work. The real answer passes through the canonical Session and domain admission path.

The active parent can receive real physical-entry followups as steering and publish an intermediate reply while keeping its original child wait. A task-question answer reads the original input actually applied before that exact tool's canonical cause; it retains that recording's presentation ID, socket and accepted sequence. It cannot borrow the initial request, an unapplied queue item or a later presentation as answer authority.

## Lifecycle, controls and retirement

Planning, running, paused, waiting-input and terminal states use the Engine state machine and optimistic versions. Each dispatch rechecks dependencies, Step version, active Attempts and retry time under the admission writer. One Execution has one active scheduling loop.

Execution Pause stops new dispatch. Already admitted Attempts retain their Sessions, Turns, links, reservations and original runtime owners for observation and settlement. Pause neither freezes native operations nor fabricates review failures or cancellation. Resume does not replay completed Attempts or settled effects. Native Turn pause/checkpoint/resume remains a separate exact-owner contract.

Late receipts cannot reopen a terminal result. Explicit retry, adopt, add or replan commands require their original version/state fences; cancelled work never reopens. Restart follows the original lease, Attempt and receipt. Missing or unknown outcomes do not authorize automatic business retries.

Step stop selects `task`, `delegated_work` or `task_and_delegated` explicitly. Ownership comes from the original caller Step/Attempt and committed `added_step_ids`, not DAG edges. The original writer commits the cancellation fence before signaling the exact owner. Ordinary control FIFO cannot delay User Stop. Unrelated siblings may finish, while cancelled scopes cannot dispatch, retry or delegate again. Whole-Execution cancellation remains an explicitly different command.

Durable question answers, Steer and StopTurn use their existing Attempt write-ahead effects. The original owner retains delivery through observer loss and confirms only the exact original command. Slow controls do not block other Attempts' settlement or a direct user stop. Canonical Turn cancellation settles its exact Attempt/Step without being promoted to aggregate cancellation.

Runtime retirement requires the actual original cleanup witness. An inactive Link, terminal-looking status, missing owner/cache entry or checkpoint is insufficient. Background processes, user Browser state, legitimate writebacks and unknown effects keep their real owners. Cleanup retries release original resources without rerunning Agent work. History is not a permanent runtime reference.

AutoWork cancellation also fences its exact Requirement claim generation, execution creation and dispatch. Restart may finish the already-authorized cancellation from those facts, never choose another claim or manufacture an Attempt. Accepted cancellation, actual stop and confirmed resource retirement remain separate observations.

## Activity and process presentation

The activity API aggregates existing Session, Step/Attempt, Cron, AutoWork, Remote and resource-owner facts. Reads and polling never acquire a Runtime or start work. Stop commands route exact captured targets to their domain owners; a transient stop-all snapshot is not a second execution ledger. Single task, delegated ownership, DAG dependencies and whole Execution remain distinct scopes.

The shared chat timeline displays accepted delegation and real Step terminal records. An accepted Step is not proof of startup; an Attempt ending, retry, loop or parent Turn ending is not Step completion. Adjacent records from the same original request/tool Effect and semantic kind may compact in presentation, while every record retains its exact identity and navigation target. Historical terminal entries pin the original Attempt; current-task control requires an explicit selection.

`GET /api/conversations/{session}/delegation-records` reads source-owned Execution events within the loaded accepted-input window. Session and Execution sequences are never compared across streams. Pagination pins original event bounds, and asynchronous completion does not require another parent message. Opening a record cannot start, resume or retry an Agent.

## IDMM and validation

[IDMM](../../crates/backend/nomifun-idmm/src/lib.rs) reads bounded verified observations through its Session port. It owns no Runtime, Session or tool authority. Rule or explicitly configured side-model decisions retain the exact source question; credentials, payment, destructive choices and ordinary permissions cannot be auto-approved through that route. Failed parsing and stale questions preserve failure/wait state rather than creating arbitrary input.

Targeted execution validation covers real overlap, dependency joins, parent integration, repeated proposals, version races, exact cancellation, native child-wait recovery, budgets, pending/unknown effects, question continuation and resource retirement. Run `bun run check:agent-session-boundary`; Session content changes also run affected canonical history, reset and consumer tests. Controlled local model fixtures do not establish real-provider, device or other-platform support.
