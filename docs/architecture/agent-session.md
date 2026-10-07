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

`engine_history.rs` reads typed canonical events and resolved payloads. It does not read Message projections to determine model roles. Native closed turns replay their structured journal. Missing Runtime records fail closed rather than falling back to chat text.

An accepted input that fails or is cancelled before Runtime startup remains valid context together with its explicit terminal fact. It does not acquire fabricated Runtime, tool or completion evidence.

Creation prompts, Cron notices and Execution summaries enter context through formal message events. Text before an explicit Agent transition is data only; it does not carry old grants, handles or completion gates. Context-clear events and the fixed accepted-root cursor bound the window.

Current-generation history, immutable Snapshots, model changes and explicit Agent transitions remain supported product semantics. They are verified against identity, sequence, binding and content integrity. Executable recovery requires the native checkpoint's exact build and binding. Closed read-only event history may cross an explicitly validated transition without gaining authority.

Session reasoning uses one native `reasoning_effort` field defined by the shared contract. No lossy mirrors or fallback columns are maintained.

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
creation fail closed while canonical running precedes a proven gate. User profiles use owner/Session identity under
`browser-v4/agent-sessions/<hash>/`, independent of grant definitions. Session deletion includes this browser and its
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
Bodies and resources use the same native frozen context reader; Skill hooks, shell, and forks do not
execute or grant tools. Tool search is a built-in Runtime facility.

## Consumers and deletion

UI consumes current stream and Message projections directly. It has no marker-dependent local terminal-processing state machine or prose-based reclassification of old errors. Channel binds through its current owner; orphan rows without an authority binding produce a conflict instead of being automatically rebound.

Error diagnostics capture the Agent, model and workspace admitted for the exact Turn and retain them with the original detail in the existing canonical terminal payload. The backend emits typed `taskIncompleteReason` for incomplete tasks; the renderer never guesses the cause from diagnostic prose. Live errors and historical projections consume the same terminal information. Missing historical fields remain unrecorded rather than borrowing the Session's current selection. User-facing reasons and recommended actions are separate from collapsed technical details.

One `ModelFailureDiagnostic` carries model-request refinements and evidence from invocation through the Broker, typed failure and existing terminal into `providerDiagnostic`; classification is never reconstructed from English messages, UI projections or unknown response bodies. Explicit upstream machine codes take precedence over broad status. When HTTP 401, 403, 404 or 429 alone cannot prove credential expiration, model access, an incorrect API path or depleted balance, the explanation retains that uncertainty. Diagnostics capture the sanitized actual endpoint, provider and model, protocol/auth method, HTTP status, upstream code/type/parameter, request ID and retry hint; arbitrary response prose, credentials and authentication-header values are excluded. Safe local transport causes remain available in technical details, without changing retry, authorization or settlement rules. Failed-attempt identity after failover is shown separately from the Session-selected model; account links require trusted configuration matching the recorded provider identity.

Failures, current pauses and rejected requests share one error presentation. Model or provider failures automatically unlock the composer after the existing owner proves cleanup and native resource release and commits canonical `turn/failed`; the user does not need to end the Turn again. The same owner may settle an already-saved current-generation typed model failure pause during startup, Session reads or the next admission. Its transaction rechecks the exact pause revision, execution fence, Snapshot, current-generation cleanup receipt and absence of pending/unknown effects. It never runs a model, replays a tool, or changes failure into success or cancellation. Unproven cleanup, unknown effects, manual pauses and other recovery pauses retain their native fences and explicit recovery authority. Genuine pause notes derive only from verified current state and never enter Message history or invent failed terminals. Rejected requests remain transient within their originating Session, and an admitted request's canonical error replaces its HTTP fallback.

Cron, Companion, Requirements, AutoWork, IDMM and AgentExecution retain their own business configuration and supervision state. Session input, admission, cancellation and completion use canonical Turn receipts.

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

A Step spec is task input rather than a second artifact contract. The sole Runtime enforces typed requirements, completion and delivery; the scheduler cannot infer another acceptance gate from reference filenames, formats or counts in prose. Recovery uses the Session owner's complete canonical OperationId and the same output and error classification as normal settlement. Terminal metadata cannot become an output summary. Missing, unknown or incomplete receipts do not authorize replay; automatic retry requires an explicit structured retry permission.

Automatic tool routing suggests actions for explicit current-input intent without truncating the authorized catalog. Delegated input classifies only step_spec, excluding task_brief and reference content. Tool discovery searches both advertised and deferred authorized tools; exposing a schema earlier does not add authority.

Cancellation fixes its target Turn in the canonical owner's transaction. Runtime cancellation uses only that receipt's target and native execution generation. Replays and delayed StopTurn commands cannot cancel a successor. Execution persists a stable cancellation identity and exact target OperationId in its existing write-ahead intent. Decision continuations share ordinary scheduling jobs so other results, cancellation and lease loss remain responsive.

The refactor uses an Agent-only clean cut in the same database. Retired Agent sessions, messages, logs, Execution, presets and bindings are neither imported nor read. Non-Agent configuration follows its explicit owner and cutover contract. Repository cleanup must not operate on a developer's or user's live data directory.

Replacement includes physical deletion of the old reader, writer, DTO, schema fields, fixtures, tests and documents. Published migration lineage needs its own deliberate boundary; checksum rewriting and permanent dual writes are not substitutes for deletion. Unused generic checkpoint APIs, historical deletion manifests and generator requirements for obsolete documents are removed.

## Sources and validation

- [Session owner](../../crates/backend/nomifun-conversation/src/canonical_session_owner.rs) and [Store](../../crates/backend/nomifun-agent-session/src/store.rs)
- [Official Runtime](../../crates/backend/nomifun-app/src/router/official_runtime.rs) and [Runtime implementation](../../crates/backend/nomifun-agent-runtime/src/lib.rs)
- [Journal](../../crates/backend/nomifun-app/src/router/engine_journal.rs), [canonical history](../../crates/backend/nomifun-app/src/router/engine_history.rs) and [native replay](../../crates/backend/nomifun-app/src/router/unified_runtime_history.rs)
- [Contracts](../../crates/backend/nomifun-agent-contracts/src/lib.rs) and [schema](../../crates/backend/nomifun-db/migrations/001_canonical_baseline.sql)

Run targeted Rust/UI tests, contract generator check, `bun run check:agent-session-boundary` and `bun run check:uarc-boundary`. Renderer changes also require `bun run check:desktop-ui-boundary`; the minimum supported desktop viewport is 880×600.

Cover new and sequential turns, rejected admission followed by a new turn, Agent switches, context clear, cancellation, restart, pause recovery, unknown effects and deletion. Missing structured history must not be repaired by a projection fallback.
