I have verified everything against real code. Here is the corrected design.

---

# B1 — DURABLE RESUMABLE ROUND (corrected)

## 0. What changed, and why each change was forced

| Rejected mechanism | Verified failure | Replacement |
|---|---|---|
| anchor on `EditableTurnCheckpoint.start_len` | `run_compaction` sets `self.messages = result.messages; self.editable_turn = None;` (`engine/mod.rs:2461-2462`) and is awaited **inside** the turn loop (`:1363`, loop opens `:1330`). Worse: `auto::autocompact` returns **exactly two** messages — a boundary + a summary (`compact/auto.rs:207-211`) — so the root user message is *physically gone*, not merely re-indexed. | **`self.messages.pop()`** of the assistant message this pass pushed at `:1906-1907`, plus an **owned `Vec<ContentBlock>` requirement** held in a local of `execute_turn_inner`. No offset exists anywhere. |
| `RoundLedger.requirement: String` | drops `ContentBlock::Image`, which `execute_turn_inner` explicitly accepts (`:1260-1268`) and pushes verbatim (`:1317`) | requirement is `Vec<ContentBlock>`, re-pushed verbatim; the *durable* ledger stores only a digest + progress (never base64) |
| requirement restated in the system prompt **and** re-pushed | duplicates the whole RAG/prelude blob (`manager/nomi/agent.rs:1952-1988` prepends prelude + knowledge hits before `run_content.push`), and made the proposal's own "exactly once" test unsatisfiable | the system section carries **only** round facts. The requirement appears exactly once, as the tail user message. |
| `ContextContributor` + `Arc<Mutex<..>>` + new `AgentEngine` field | `resume_with_provider` hardcodes `context_contributors: Vec::new()` (`:718`), so a resumed engine would never render it; and the field breaks 7 struct literals | inline assembly beside `append_system_resource_context` (`:1445-1448`) from `&self`. **Zero new `AgentEngine` fields.** |
| host gate `committed_artifact_count == 0` | does not exist in `nomifun-ai-agent` (only `nomifun-conversation`: `stream_relay.rs:208,2161,2525`; `service.rs:8797,9045`; `relay_error_code.rs`), and counts image receipts only | **the adjudication already moved** — the working tree has A1 in `relay_error_code.rs` (`incomplete_stop_code(outcome.stop_reason)`, `turn_succeeded(&outcome, ..)`) consumed at `service.rs:9035-9070`. B1 adds **no** host terminal branch for cap exhaustion. |
| `TURN_OUTPUT_CEILING` fixed lifecycle const | unreachable on an `Error` terminal (`map_turn_failure`'s Error arm derives from `AgentErrorCode`) | A1 already added `OUTPUT_TRUNCATED` on the *Finish* path where it is reachable. B1 adds one `AgentErrorCode` variant for its own distinct verdict. |
| `StopReason::Refusal`, `TokenUsage.reasoning_tokens` | `StopReason` has 4 variants (`message.rs:104-115`) with 3 exhaustive matches; `TokenUsage` has 30+ exhaustive literals | **cut from B1.** Named as A2/C1 with the site counts. |
| `truncation_continuation_prompt` unit tests deleted | they do not exist (repo-wide grep: definition `agent.rs:720`, call `agent.rs:2071`, nothing else) | corrected delete list below |

---

## 1. Blocker 1 — the anchor that survives compaction

### 1.1 The mechanism (no `start_len`, no `editable_turn`)

Two independent pieces:

**(a) The drop.** At `engine/mod.rs:1906-1907` the engine itself pushes `Message::now(Role::Assistant, assistant_content)`. Between that push and the natural-termination block at `:1909` nothing mutates `self.messages` (`:1909` is the `if`, `:1910` resets the stagnation guard, `:1915` clones). Therefore `self.messages.pop()` at the top of that block removes **exactly** the truncated draft — independent of compaction, of history length, of prior rounds. `supersede_written_draft` (`:1898`, defined `:2990`) cannot have altered it, because it only fires when the same round produced a file write, and `StopReason::MaxTokens` with non-empty `tool_calls` is already a hard protocol violation at `:1754-1756`.

**(b) The requirement.** `let round_requirement: Vec<ContentBlock> = user_content.clone();` taken in `execute_turn_inner` immediately **before** `self.messages.push(Message::now(Role::User, user_content))` at `:1317`. A stack local of the very function that owns the loop.

### 1.2 The three proofs

| Hazard | Why the anchor survives |
|---|---|
| `run_compaction()` (`:2418`, called `:1363`) | It touches `self.messages`, `self.editable_turn`, `self.compact_state` and nothing else. `round_requirement` is a local; the ledger is in `self.host_context`, which `run_compaction` never reads or writes. The pop targets `self.messages.last()`, which is re-established by *this pass*, after compaction ran. Post-compaction transcript for round 2 is `[boundary, summary, requirement]` — correct and desirable. |
| in-turn error rollback (`:1187-1197`) | `self.messages = safe_messages` restores only messages; `host_context` is deliberately outside it. The restart hook re-sets `*safe_messages = self.messages.clone()` **after** popping and re-pushing, so a round-2 provider error rolls back to the round-2 floor, never into round-1 prose. (`safe_messages` is a local of the wrapper at `:1160` — only in-engine code can move it, which is an independent reason the loop cannot live in the host.) |
| process restart | `self.host_context` is mirrored into `session.host_context` by `save_session` (`:2563`), restored by `resume_with_provider` (`:684` → `:723`, filtered only for `editable_turn`), and the backend uses the same `SessionManager` (`nomi_session_persistence.rs`). The requirement itself is **not** persisted by the engine and does not need to be: the host's `turn_delivery_request_payload` is the durable authority for `{content, files}` and rebuilds `image_blocks` on the next send. |

### 1.3 What is durable, and the fence

One key, engine-owned:

```
nomi.round.ledger = <RoundLedger as compact JSON>
```

`session.rs:59-64`'s doc comment is amended (no type change) to state the prefix contract: `nomifun.*` host-owned (today's only key, `manager/nomi/agent.rs:254`), `nomi.*` engine-owned. Additive, not a dual-read alias.

**Fence = content-addressed, not id-addressed.** `RoundLedger.requirement_digest = sha256_hex(serde_json::to_vec(&user_content))`. At turn start:

- digest matches a persisted ledger → **adopt** its `steps`/`effects`/`cutoff`, and **reset `attempt` to 1**. This is the same requirement being attempted again (process kill, or D1's Continue re-sending the original payload). A new accepted turn always gets a full attempt budget.
- digest differs, or the JSON does not parse → `self.host_context.remove("nomi.round.ledger")`. Fail-closed; a corrupt or foreign ledger degrades to "first round", never to garbage in a system prompt.

A `source_message_id` fence would have been *wrong*: after a process restart the host re-sends with a new durable message id, which is exactly when the carry-forward is needed. The digest is deterministic because `user_content` is validated to contain only `Text`/`Image` blocks (`:1260-1268`), both fixed-field structs.

**Reap rule — one site.** In the wrapper `execute_turn_with_content_for_source_and_tool_allowlist`, after `execute_turn_inner` returns (`:1178`), before the redaction branch:

```rust
if let Ok(result) = &result
    && matches!(result.stop_reason, StopReason::EndTurn | StopReason::ToolUse)
{
    self.clear_round_ledger();   // host_context.remove + save_session
}
```

One site covers all 5 `Ok` exits (`:1287, 1337, 1978, 2290, 2305`) and all 28 `Err` exits. A ledger survives `MaxTokens`, `MaxTurns`, and `Err` **on purpose** — that is unfinished work. `abort_current_turn` (`:2661`) gets the same call (its wrapper future was dropped). `clear_context` (`:2605-2612`) and `init_session` (`:815`) already clear the whole map.

**Two leaks the old proposal called "free" and were not:**
- `record_host_text_turn` overwrites `editable_turn` unconditionally at `:1222-1226`, snapshotting `prior_host_context: self.host_context.clone()`. If a ledger is live, `rewind_last_turn`'s `self.host_context = checkpoint.prior_host_context` (`:2646`) *restores* it.
  **Fix:** both snapshot sites (`:1225`, `:1314`) capture `self.host_context_without_round_state()`, and `rewind_last_turn` calls `clear_round_ledger()` after `:2646`. Destruction becomes explicit; the digest fence stays as defence in depth. `rewind_last_turn`'s three actions are otherwise unchanged, and `rewind_last_turn_truncates_to_marker` (`set_config_tests.rs:1389-1421`, asserting `engine.host_context == prior_host_context` with the real `nomifun.image_generation.route` key) stays green.
- `set_host_context_value` (`:833-843`) ends in `save_session()`, which clones the whole message vector and rewrites the session file + index (`:2557-2574`). The ledger is therefore mutated through a **private** helper that touches `self.host_context` directly and does **not** save; persistence rides the existing save points (`:2283` per tool batch, and the restart hook's own save). Net extra disk writes on a turn that never truncates: **zero** — the ledger is not even materialized into `host_context` until the first restart.

---

## 2. Blocker 2 — altitude, and the turn-counter multiplication

**The loop lives in the engine**, as a fourth hook at the natural-termination point, placed **first** (immediately after `*safe_messages = self.messages.clone()` at `:1915`, before the steering / goal / spec-recheck hooks at `:1926-1975`). A truncated pass is not a completed assistant response, so the three existing hooks — all of which assume one — must not act on it.

Justification that survives the model-only objection:

**The round does NOT consume a turn.** `apply_model_only_ceiling` sets `overrides.max_turns = Some(1)` (`factory/nomi.rs:49`) for every non-owner runtime (`:138-140`), so `limit == 1` (`:1334`) and every existing hook's `turn + 1 < limit` gate is `1 < 1` → false. The restart hook therefore does **not** gate on `turn + 1 < limit` and does **not** increment `turn`. It increments its own `attempt` and `continue`s. Consequences:

- model-only sessions get exactly the 3 passes they get today (`MAX_ROUND_ATTEMPTS = 3`), not 0 — no regression;
- the multiplication bug is structurally gone: the host no longer re-enters `execute_turn_inner`, so `let mut turn: usize = 0` (`:1323`) is never reset. Two host continuations used to permit `3 × max_turns` provider passes; the host's own comment at `agent.rs:2052-2056` concedes the mechanism ("starting a fresh pass resets the engine's loop guard");
- termination is bounded solely by `attempt < MAX_ROUND_ATTEMPTS` — `attempt` is a monotonic local of `execute_turn_inner`, so at most 2 restarts per turn regardless of `max_turns`;
- `stagnation_guard`, `tool_retry_tracker`, `routed_tool_calls_seen`, `ToolEfficiencyStats` and the `*_nudged` once-per-turn flags keep counting across rounds instead of silently resetting;
- `total_usage` accumulates per pass at `:1825-1828`, so the bill stays correct even though `turn` does not move;
- **every host is covered by one hook**: `nomi-cli` (`main.rs:286, 354, 632`), delegated subagents (`local_agent_invocation.rs:618`), provider health checks (`services/provider_health.rs:285`) and the backend (`manager/nomi/agent.rs:2008`) all funnel through `execute_turn*`. The host loop reaches only the last one.

```rust
/// Total attempts at one accepted requirement, including the first. 3 preserves
/// today's envelope (MAX_TRUNCATION_AUTO_CONTINUES = 2 → 3 passes → the observed
/// output_tokens = 24576 = 3 × 8192) while making all three passes useful.
pub const MAX_ROUND_ATTEMPTS: usize = 3;
```

---

## 3. Steering is never deleted; images survive

**Steering.** `truncate(start_len)` deleted accepted `Role::User` interjections that `drain_steering` had already `q.drain(..)`-ed out of the inbox (`:881-889`) and that the host race-tail may have re-supplied under the same `source_message_id` (`agent.rs:2023-2042`). The corrected mechanism **removes exactly one assistant message and never a user message**, so:

- every already-drained steer stays in the transcript, structurally;
- the restart hook additionally calls `self.drain_steering()` itself and appends each interjection as a trailing `Role::User` text message *after* the re-pushed requirement, so a steer that arrived during the truncated pass is delivered on the very next pass instead of waiting. Draining is unconditionally safe here (unlike `:1926`, which guards `turn + 1 < limit`) because the hook does not increment `turn`, so a provider pass is guaranteed to follow;
- across a **process restart**, steering is not the engine's to preserve: `drain_steering` reads a host-owned `Arc<Mutex<VecDeque<String>>>` (`:881-889`) and any already-appended steer is in `session.messages`, persisted by `save_session` (`:2559`). The restart hook's `save_session()` runs after the appends, so an interjection is durable before the next provider await — the same rule `:1319-1321` states for the root user message.

**Multimodal.** The re-push is `Message::now(Role::User, round_requirement.clone())` — the verbatim `Vec<ContentBlock>`, `Image` blocks included. To keep exactly one live copy of each image on the wire, the hook first calls the existing, tested `self.redact_user_images_since(0)` (`:2381-2411`), which replaces prior user `Image` blocks with `USER_IMAGE_HISTORY_PLACEHOLDER`. `abort_current_turn` already uses `redact_user_images_since(0)` with the identical rationale ("compaction may legitimately clear that anchor while a run is still in flight", `:2701-2704`). Invariant after a restart: the tail user message is the only holder of live image payloads.

Two consecutive `Role::User` messages (tool results, then the requirement) are an already-supported shape: `auto::autocompact` returns two consecutive user messages (`compact/auto.rs:207-211`), the steering hook pushes N in a row (`:1931-1935`), and the Anthropic family collapses them under `compat.merge_same_role()` / `ensure_alternation()` (`anthropic_shared.rs:128-145`).

---

## 4. Blockers 5a and 6 — the exact restart predicate

The hook is reached only when `stop_reason == StopReason::MaxTokens && tool_calls.is_empty()` (non-empty `tool_calls` with `MaxTokens` is rejected at `:1754-1756` before the assistant message is pushed, so the old proposal's fourth refusal row was dead logic).

```rust
// engine/mod.rs, first hook inside `if tool_calls.is_empty()` (after :1915)
let restart = stop_reason == StopReason::MaxTokens
    && round.attempt < MAX_ROUND_ATTEMPTS
    && tools_advertised                       // captured at :1397, see below
    && (!truncated_calls.is_empty()           // a tool call was literally cut off
        || round.ledger.has_open_plan()       // model declared a plan with a pending/in_progress step
        || round.ledger.effects_total > 0);   // this turn already changed state
```

Each clause is a machine fact, none is prose:

| Clause | Source of truth |
|---|---|
| `tools_advertised` | `let tools_advertised = !tools.is_empty();` captured at `:1397` beside `ProviderToolAuthority::from_request_tools(&tools)`, before `tools` moves into `LlmRequest` at `:1457`. Excludes provider health checks (`provider_health.rs:285`) and any no-tool request by construction. |
| `truncated_calls` | the new `LlmEvent::ToolUseTruncated` (§7), a per-pass local declared beside `let mut stop_reason` at `:1471` and therefore reset every pass |
| `has_open_plan()` | `StepStatus::Pending \| InProgress` in the last accepted `update_plan` snapshot (§6) |
| `effects_total` | count of dispatched `Edit`/`Exec`/`Mcp`/`Irreversible` tool results this turn (§6) |

**This is how "prose IS the deliverable" stops being a hard error (Blocker 6).** A plain long answer has no truncated call, no plan, and no state-changing effect. The predicate is `false`; the engine returns `Ok(AgentResult { stop_reason: MaxTokens, .. })` exactly as today; the durable assistant text row is preserved (`stream_relay.rs:3748-3766`, documented "Preserve the already-durable raw text row"); and A1's `incomplete_stop_code(Some(MaxTokens)) → OUTPUT_TRUNCATED` marks the receipt failed-but-**retryable** (`fixed_code_retryable` returns `true` for it) so D1 can offer Continue. No hard error, no lost text, no burned budget on three identical shots at the same wall.

**Honest scope statement.** In the observed production trace round 1 had zero tool calls, no `update_plan` and no effects, so the predicate is `false` and B1 does **not** auto-restart it. That is deliberate: restarting an identical request against an identical ceiling reproduces the identical result. B1's automatic restart fires when there *is* carry-forward — which is precisely the case today's append-prose hack corrupts. For the zero-progress shape the fix is A1 (honest receipt) plus C1 (a real ceiling); B1 supplies the mechanism that D1's user-initiated Continue reuses, where spending more budget is the user's explicit decision.

### 4.1 The hook, in full

```rust
if restart {
    // The provider hit its output ceiling mid-composition. Continuing a
    // truncated draft is not recoverable: restart the round against the
    // ORIGINAL requirement, carrying the machine-built ledger forward.
    // `turn` is deliberately NOT incremented — a round is a retry of this
    // turn, not another tool-loop iteration, and model-only sessions run at
    // max_turns = 1.
    let Some(dropped) = self.messages.pop() else { /* unreachable */ };
    debug_assert_eq!(dropped.role, Role::Assistant);
    round.attempt += 1;
    round.ledger.cutoff = std::mem::take(&mut truncated_calls);
    self.redact_user_images_since(0);
    self.messages.push(Message::now(Role::User, round.requirement.clone()));
    for text in self.drain_steering() {
        self.messages.push(Message::now(Role::User, vec![ContentBlock::Text { text }]));
    }
    self.persist_round_ledger(&round);          // host_context, no extra save
    // Fail-closed cleanup of any published-but-unsettled tool card and reset of
    // the per-turn citation buffer, via the trait method the engine owns.
    self.output.emit_stream_start(&self.current_msg_id);
    *safe_messages = self.messages.clone();     // round N's rollback floor
    self.save_session();
    tracing::warn!(target: "nomi_agent", attempt = round.attempt,
        max_attempts = MAX_ROUND_ATTEMPTS, dropped_draft_bytes = assistant_text.len(),
        steps = round.ledger.steps.len(), effects = round.ledger.effects.len(),
        cutoff = round.ledger.cutoff.len(),
        "output ceiling reached; restarting the round against the original requirement");
    continue;
}
```

`emit_stream_start` is safe on every sink and is the exact replacement for the host-only `truncate_active_tool_calls_for_auto_continue`: `BackendOutputSink::emit_stream_start` (`backend_output_sink.rs:2807-2822`) already calls `fail_active_tool_calls` and clears `turn_text`, and the host documents a repeat Start under the same msg_id as benign for the UI (`agent.rs:2035-2039`). `NullSink`, `terminal.rs:38`, and `protocol_sink.rs:97` all handle it.

**One thing this design does NOT claim.** `stream_relay.rs:2135`'s `full_text_buffer` accumulates every `Text` event for the whole turn and `finalize` turns it into `outcome.final_text` (`:3760-3766`); a `Start` event is not in the relay's event match, so round 1's visible prose remains part `final_text` if a later round finishes cleanly. That is today's behaviour under the host loop too — B1 does not regress it, and A1 already suppresses writeback and `result_ok` for the `MaxTokens` case (`service.rs:9056-9070`). The visible-boundary event is **D1's**, and the seam is exactly `full_text_buffer` at `stream_relay.rs:2135`. The old proposal's claim that "the runaway prose leaves the pipeline" was false for the durable text and is dropped.

---

## 5. Blocker 5b — the restarted round that lies

**Exact predicate:** *a turn that consumed more than one round and produced zero successful state-changing tool effects is a failed turn, regardless of its final stop reason.*

```rust
result.rounds > 1 && result.effects_ok == 0
```

`rounds > 1` means the restart predicate fired, which already required machine evidence of intended tool work. So a 40-token `"Created miniapp.html."` returning `StopReason::EndTurn` — which A1 cannot catch, because `incomplete_stop_code(Some(EndTurn))` is `None` and `final_text` is non-blank (`relay_error_code.rs:99-129`) — is caught here. The engine's own `unbacked_completion_claim` cannot catch it either: it bails at `let first_write = first_write?;` (`:2851`) when no tool ever ran.

**Carrier.** `AgentResult` (`engine/mod.rs:3085-3091`) gains two fields:

```rust
#[derive(Debug)]
pub struct AgentResult {
    pub text: String,
    pub stop_reason: StopReason,
    pub usage: TokenUsage,
    pub turns: usize,
    /// Attempts at the accepted requirement, including the first. 1 = no restart.
    pub rounds: usize,
    /// Successful state-changing tool effects across every round of this turn.
    pub effects_ok: usize,
}
```

**Terminal.** In `manager/nomi/agent.rs`, in the `Ok(agent_result)` arm, immediately after `let stop_reason = map_engine_stop_reason(..)` and the `info!` (i.e. before `fail_active_tool_calls` at `:2132`), reusing the established, tested "returned but the obligation is unmet" pattern (`:2204-2223`):

```rust
if agent_result.rounds > 1 && agent_result.effects_ok == 0 {
    let send_error = unproductive_round_to_send_error(&agent_result);
    let stream_error = send_error.stream_error().clone();
    self.backend_output_sink.fail_active_tool_calls(
        "The turn restarted after the output ceiling and never completed a state-changing tool call.");
    self.backend_output_sink.abort_artifact_delivery_turn();
    term_guard.terminalize(move |runtime, turn| {
        runtime.emit_error_data_for_turn(turn, stream_error)
    }).await.map_err(AgentSendError::from_app_error)?;
    return Err(send_error);
}
```

`AgentSendError::new(msg, AgentErrorCode::UserLlmProviderNoProgressAfterRestart, AgentErrorOwnership::UserLlmProvider, detail, /*retryable*/ false, /*feedback*/ false, resolution(ChangeModel, Some(ProviderSettings)))` — same helper shape as `send_error.rs:43-56`. `map_turn_failure`'s `Error` arm derives the receipt token from the serde name (`relay_error_code.rs:164-172`), so the receipt records `user_llm_provider_no_progress_after_restart`, `result_ok = 0`, and writeback is skipped because it is gated on `RelayTerminal::Finish` (`service.rs:9058-9070`). Like the existing artifact-failure Error path this skips `commit_verified_turn_if_not_cancelled` and therefore the browser-lane close at `agent.rs:2477-2480`; that is consistent with the pattern being copied.

**Subagents get the same verdict**, in the same place they already reject `MaxTokens`/`MaxTurns` — `map_agent_invocation_outcome` (`local_agent_invocation.rs:559-577`) adds one arm before the `match result.stop_reason`:

```rust
let incomplete = if result.rounds > 1 && result.effects_ok == 0 {
    Some("restarted after its output ceiling and never completed a state-changing tool call")
} else { match result.stop_reason { /* unchanged */ } };
```

---

## 6. Ledger population — machine truth only, and every reused type is `pub`

Types reused, each verified:

| Type / fn | Path:line | Verified |
|---|---|---|
| `update_plan` module | `nomi-tools/src/lib.rs:19` | `pub mod update_plan` |
| `UpdatePlanArgs { explanation, plan }` | `nomi-tools/src/update_plan.rs:36-42` | `pub`, `Serialize + Deserialize` |
| `PlanItemArg { step, status }` | `update_plan.rs:29-33` | `pub`, `Serialize + Deserialize` |
| `StepStatus` (snake_case) | `update_plan.rs:20-26` | `pub`, `Copy + PartialEq + Eq`, serde `snake_case` |
| `ToolCategory{Info,Edit,Exec,Mcp,Irreversible}` | `nomi-protocol/src/events.rs:98-110` | `pub`, `Copy`, already imported at `engine/mod.rs:10` |
| `Tool::category_for(&Value)` | `nomi-tools/src/lib.rs:255-257` | trait method, defaults to `category()` |
| `Tool::describe(&Value)` | `nomi-tools/src/lib.rs:288-294` | trait method; the **default dumps the whole input JSON**, hence bounding is load-bearing for MCP tools (`Write`/`Edit`/`Bash`/`Read`/`Grep`/`Glob`/`ApplyPatch`/`ExecCommand`/`WriteStdin` all override) |
| `ToolRegistry::get(&str) -> Option<&dyn Tool>` | `nomi-tools/src/registry.rs:528-533` | `pub` |
| `truncate_middle`, `TruncationBudget::Bytes` | `nomi-tools/src/output_truncation.rs:12-31`, re-exported `lib.rs:61` | `pub`. Signature is `truncate_middle(&str, TruncationBudget) -> String` — the old proposal passed a bare `usize`, a type error. |
| `sha2` | root `Cargo.toml:182`, already used by `nomi-tools` | one-line `sha2.workspace = true` in `nomi-agent/Cargo.toml`; no new vendor |

**Producer A — `update_plan` snapshots.** The host parses the tool *result* at `backend_output_sink.rs:673-683` (`fn parse_plan_entries`, private) and discards the snapshot at its call site `:2537-2559` after emitting `AgentStreamEvent::Plan`. The engine does not touch that path and does not need `parse_plan_entries`: it already holds the raw `ContentBlock::ToolUse { input }`. Gate on `!is_error`, because `update_plan` returns `ToolResult::error` for invalid args (`update_plan.rs:139-142`) and for an empty plan (`:146-147`), and neither may clobber a good ledger. Full-snapshot semantics: **replace**, never merge. `UpdatePlanTool::category()` is `ToolCategory::Info` (`:132-134`), so Producer B cannot double-count it.

**Producer B — the effect log.** For each result whose call's `category_for(input)` is `Edit | Exec | Mcp | Irreversible`, push `{ tool, label: truncate_middle(&t.describe(input), TruncationBudget::Bytes(160)), ok: !is_error }`, bounded to 24 entries oldest-dropped. A successful `Read` is not progress; a `Write`/`Bash` is. `effects_ok` = count with `ok == true`.

**Placement — the borrow hazard the old proposal walked into.** `artifact_identity` at `:2066-2070` borrows `self.tools` and stays live through the `emit_tool_result_with_images_and_context` call ending at `:2143`, so any `&mut self` call inside `for result in &mut outcome.results` (`:2047-2148`) is a borrow-check error; and that loop's `find_map` at `:2055-2065` binds only `id, name`, never `input`. Both problems vanish by running the ledger pass **after** the loop, immediately after `efficiency.observe_results(&outcome.results)` at `:2150` — the existing precedent for exactly this shape — iterating `&outcome.results` alongside `&tool_calls` to recover `input`. Reading `is_error` there is also *more* correct: it reflects the `ToolMediaDelivery::Failed` adjustment at `:2134-2142`.

**No third producer.** No transcript scraping (that is what produced the phantom `Read` in the observed trace), no self-summarization pass (another generation against the same ceiling), no filesystem probing.

**Encoding.** The mutation path is read → `serde_json::from_str::<RoundLedger>` → mutate → `to_string` → `self.host_context.insert(..)`, with no cached second copy. `host_context` stays `BTreeMap<String, String>` (load-bearing: `#[serde(default, skip_serializing_if)]` at `session.rs:62-64`, cloned wholesale into `EditableTurnCheckpoint.prior_host_context` at `session.rs:19-22`, compared for equality by `set_config_tests.rs:1418`). At ≤4 KB the re-serialize cost is negligible next to `save_session`'s full transcript clone, and it removes the two-sources-of-truth hazard entirely.

**New types** (`crates/agent/nomi-agent/src/round.rs`, new file). `RoundLedger` derives `Debug, Clone, Default, PartialEq, Serialize, Deserialize` — safe, because it holds only `String`/`usize`/`bool`/`StepStatus`. `RoundState` (not serialized) holds `requirement: Vec<ContentBlock>`, `attempt: usize`, `ledger: RoundLedger`; it deliberately derives **no** `PartialEq`, because `ContentBlock` does not derive it (`nomi-types/src/message.rs:10-11`: `Debug, Clone, Serialize, Deserialize` only) — which is also why locating the root message by value-equality is not an option and the pop-the-last-assistant-message anchor is.

**Rendered system section**, appended by a new file-level `fn append_round_context(system: String, section: Option<String>) -> String` placed beside `append_system_resource_context` (`engine/mod.rs:252-276`) and called at `:1448`, immediately before `self.cache_detector.record_request(&system, &tools)` at `:1451`:

```
[resumable round 2/3] Your previous attempt was cut off by the provider's output
token ceiling. That draft has been REMOVED from your context and cannot be
continued. The original request is restated as the last user message below.

ALREADY DECLARED (your own plan):
  [x] scaffold the toolbox layout
  [>] write miniapp.html
  [ ] verify it opens

ALREADY DONE (observed tool effects):
  ok    Bash: mkdir -p toolbox

WHAT WAS CUT OFF:
  Write (6142 bytes of arguments streamed, NOT executed)

RULES FOR THIS ATTEMPT:
- Your first action must be a tool call. Do not restate the plan in prose.
- Split any large file: write a small complete version first, then Edit/append.
```

System channel, never `Role::User`: `set_system_resource_inbox`'s doc states the rule the tree already relies on ("never into the conversation transcript as a user message", `:865-871`), the system prompt is rebuilt on **every** pass inside the loop (`:1400-1448`), and a `Role::User` append would pollute `turn_user_text`, distillation, and knowledge write-back. The requirement text is **not** repeated here — it is the tail user message.

---

## 7. Provider plumbing — provider-agnostic, one new event

Today the cut-off tool call's identity is destroyed silently in **two** families.

**(a) `nomi-types/src/llm.rs`** — add to `LlmEvent` (`:28-56`):

```rust
/// A tool call the provider began streaming but truncated at its output
/// ceiling. NEVER executable and never enters `tool_calls`. Emitted so the
/// resumable round can tell the next attempt which call was cut off and how
/// far it got, instead of discarding the accumulator in silence.
ToolUseTruncated { id: ToolUseId, name: String, argument_bytes: usize },
```

`LlmEvent` is not `#[non_exhaustive]` and derives only `Debug, Clone` — no extra bounds.

**(b) `nomi-providers/src/openai.rs`** — drain at **exactly one** site, which is the fix for the deferral invariant the file itself documents at `:1746-1750` ("compatible gateways that … deliver the final tool fragment after their first finish_reason"):

- `:1773`, the `"length"` arm: **delete** `state.tool_calls.clear();`, leave the accumulators. Safe: the arm sets `state.finish_seen = true` at `:1753`, so `infer_terminal_from_done` returns early (`:748-750`) and cannot manufacture a `ToolUse`; post-finish fragments keep appending into the same index-keyed accumulator (`:1720-1735`).
- `:719`, `drain_terminal_events`'s `MaxTokens` branch: replace `self.tool_calls.clear()` with `events = drain_truncated_tool_calls(self);`, emitted **before** `events.push(LlmEvent::Done { .. })` at `:738` — post-`Done` events are a hard protocol violation at `engine/mod.rs:1496-1507`. Draining once, at the true terminal, makes `argument_bytes` the real final count instead of a double-emitted undercount. `:685` (`poison`), `:703` and `:708` stay untouched.

```rust
fn drain_truncated_tool_calls(state: &mut StreamState) -> Vec<LlmEvent> {
    state.tool_calls.drain(..)
        .filter(|acc| !acc.name.trim().is_empty() && !acc.id.trim().is_empty())
        .map(|acc| LlmEvent::ToolUseTruncated {
            id: acc.id, name: acc.name, argument_bytes: acc.arguments.len(),
        })
        .collect()
}
```

**(c) `nomi-providers/src/anthropic_shared.rs`** — the same loss exists for Anthropic, Bedrock and Vertex, which all share `parse_sse_data`: the `"max_tokens"` arm at `:778-788` does `state.pending_tool_calls.clear(); state.reset_current_block();`. `StreamState` (`:230-270`) gains one field `truncated_tool_calls: Vec<LlmEvent>`, initialized in `StreamState::new()` (`:279-297`) — the **only** constructor (`Default` delegates at `:272-276`; repo-wide grep confirms no struct literal anywhere). The `"max_tokens"` arm converts each staged `pending_tool_calls` entry and, when `current_block_type == Some("tool_use")`, the in-flight block (`tool_id`, `tool_name`, `tool_input_json.len()`) into `ToolUseTruncated` and stashes them; `message_stop` (`:795-830`) releases them ahead of the `Done`. The executable invariant is unchanged: nothing truncated is ever dispatched.

**(d) `nomi-providers/src/gemini.rs:749-753`** — `"MAX_TOKENS"` with pending calls is a hard `ProviderError::Parse`, which never reaches the engine as `MaxTokens` at all. Set `stop_reason = MaxTokens` and have `terminal_events` (`:770-793`) emit `ToolUseTruncated` for each `pending_calls` entry instead of `ToolUse`, so Gemini participates in the same recovery rather than erroring the turn.

**(e) `engine/mod.rs`** — one arm in the event match beside `ToolUseDelta` (`:1652`):

```rust
LlmEvent::ToolUseTruncated { id: _, name, argument_bytes } => {
    // Not a tool call: never dispatched, never enters `tool_calls`. A fact
    // recorded for the next round.
    truncated_calls.push(LedgerCutoff { tool: name, argument_bytes });
}
```

with `let mut truncated_calls: Vec<LedgerCutoff> = Vec::new();` declared beside `let mut stop_reason = StopReason::EndTurn;` at `:1471` so it resets every pass. A `Vec`, not an `Option`: a provider may truncate several parallel calls, and keeping only the last would render a wrong fact into the prompt.

---

## 8. Ordered edit list

**`crates/agent/nomi-types/src/llm.rs`**
1. `enum LlmEvent` (`:28-56`) — add `ToolUseTruncated { id: ToolUseId, name: String, argument_bytes: usize }`.

**`crates/agent/nomi-providers/src/`**
2. `openai.rs` — new `fn drain_truncated_tool_calls(state: &mut StreamState) -> Vec<LlmEvent>`; delete `state.tool_calls.clear()` at `:1773`; replace `self.tool_calls.clear()` at `:719` with the drain, before the `Done` push at `:738`.
3. `anthropic_shared.rs` — `StreamState` (`:230-270`) `+ truncated_tool_calls: Vec<LlmEvent>`; init in `new()` (`:279-297`); populate in the `"max_tokens"` arm (`:778-788`) from `pending_tool_calls` and from the in-flight `tool_use` block before `reset_current_block()`; release in `"message_stop"` (`:811-816`) ahead of the `Done`.
4. `gemini.rs:749-753` + `terminal_events` (`:770-793`) — `MAX_TOKENS` with pending calls becomes `StopReason::MaxTokens` + `ToolUseTruncated` instead of `ProviderError::Parse`.

**`crates/agent/nomi-agent/`**
5. `Cargo.toml` — `sha2.workspace = true`.
6. **NEW `src/round.rs`** — `RoundState`, `RoundLedger`, `LedgerStep`, `LedgerEffect`, `LedgerCutoff`, `ROUND_LEDGER_KEY = "nomi.round.ledger"`, `MAX_ROUND_ATTEMPTS = 3`, `MAX_LEDGER_EFFECTS = 24`, `EFFECT_LABEL_BUDGET = TruncationBudget::Bytes(160)`, `fn requirement_digest(&[ContentBlock]) -> String`, `RoundLedger::{has_open_plan, effects_ok, push_effect, render_section}`.
7. `src/lib.rs:24` area — `pub mod round;`.
8. `src/session.rs:59-64` — amend the `host_context` doc comment with the `nomi.*` (engine-owned) / `nomifun.*` (host-owned) prefix contract. **No type change.**
9. `src/engine/mod.rs:252-276` — add `fn append_round_context(system: String, section: Option<String>) -> String` beside `append_system_resource_context`.
10. `src/engine/mod.rs` private helpers on `AgentEngine`: `fn round_ledger(&self) -> Option<RoundLedger>`, `fn persist_round_ledger(&mut self, &RoundState)` (no `save_session`), `fn clear_round_ledger(&mut self)`, `fn host_context_without_round_state(&self) -> BTreeMap<String, String>`, `fn open_round(&mut self, &[ContentBlock]) -> RoundState` (digest adopt-or-delete). **No new struct field.**
11. `src/engine/mod.rs:1222-1226` and `:1311-1315` — both `EditableTurnCheckpoint` literals capture `prior_host_context: self.host_context_without_round_state()`.
12. `src/engine/mod.rs:1250-1259` — in `execute_turn_inner`: `let round_requirement = user_content.clone();` before the push at `:1317`; `let mut round = self.open_round(&round_requirement);` beside the loop locals at `:1323-1329`.
13. `src/engine/mod.rs:1397` — `let tools_advertised = !tools.is_empty();`.
14. `src/engine/mod.rs:1448` — `let system = append_round_context(system, round.render_section());`.
15. `src/engine/mod.rs:1471` — `let mut truncated_calls: Vec<LedgerCutoff> = Vec::new();`.
16. `src/engine/mod.rs:1652` — the `LlmEvent::ToolUseTruncated` match arm.
17. `src/engine/mod.rs:1916` — **the restart hook**, first in the `tool_calls.is_empty()` block (§4.1).
18. `src/engine/mod.rs:2150` — the post-loop ledger pass (Producers A and B).
19. `src/engine/mod.rs:1287, 1337, 1978, 2290, 2305` — the five `AgentResult` literals gain `rounds`/`effects_ok`.
20. `src/engine/mod.rs:3085-3091` — `AgentResult` gains `pub rounds: usize` and `pub effects_ok: usize`.
21. `src/engine/mod.rs:1178` — reap in the wrapper on `EndTurn | ToolUse`; `src/engine/mod.rs:2661` `abort_current_turn` — `clear_round_ledger()`; `src/engine/mod.rs:2646` `rewind_last_turn` — `clear_round_ledger()` after restoring `prior_host_context`.
22. `src/local_agent_invocation.rs:559-577` — `map_agent_invocation_outcome` adds the `rounds > 1 && effects_ok == 0` incompleteness arm.

**`crates/backend/nomifun-api-types/`**
23. `src/agent_error.rs:14-54` — `AgentErrorCode::UserLlmProviderNoProgressAfterRestart`.

**`crates/backend/nomifun-ai-agent/`**
24. `src/image_generation.rs:208` — add `LlmEvent::ToolUseTruncated { .. }` to the existing tool-call rejection group (a no-tool image-intent request receiving a truncated call is the same protocol error).
25. `src/manager/nomi/agent.rs` — **delete** `MAX_TRUNCATION_AUTO_CONTINUES` (`:718`), `truncation_continuation_prompt` (`:720-733`), and the whole `MaxTokens` continuation block including `truncation_auto_continues` and the `warn!`-then-fall-through-to-`Finish` (`:2049-2086`). The `loop` remains for the steering race-tail (`:2019-2046`).
26. `src/manager/nomi/agent.rs:2130` — the `rounds > 1 && effects_ok == 0` terminal-Error branch + `fn unproductive_round_to_send_error(&AgentResult) -> AgentSendError`.
27. `src/capability/backend_output_sink.rs:2262-2273` — **delete** `truncate_active_tool_calls_for_auto_continue`; its sole production caller is edit 25 and its own doc says it is "normally a no-op". `:2537-2559` unchanged, with a comment that the engine independently owns the durable ledger so nobody later "fixes" the discard by adding a second writer.

---

## 9. Every site that breaks — verified by grep

**New `LlmEvent` variant — 2 exhaustive matches with no wildcard:**
- `crates/agent/nomi-agent/src/engine/mod.rs:1534-1741` (7 arms, closes at `:1741`)
- `crates/backend/nomifun-ai-agent/src/image_generation.rs:186-214` (arms at `:186, :194, :205, :208, :214`) — production code, not a test

**Verified NOT to break** (catch-all or `matches!`): `bootstrap.rs:58-62` and `:123-127` (`_ => {}`); `compact/auto.rs:227-231` (`_ => {}`); `factory/provider_config.rs:387-396` and `:432-450` (`_ => {}`); `one_shot.rs:146-166` (`_ => {}`); `nomi-providers/src/lib.rs:425-428` (`other => other`); `anthropic_shared.rs:432-437` (`(event, _) => event`) + `:438-445` (`matches!`); `bedrock.rs:476-481` (`event => event`) + `:483-490` (`matches!`); `gemini.rs:960-966` (`matches!`); `retry.rs` (constructs only).

**New `AgentResult` fields — 6 struct literals, no destructuring patterns exist:**
`engine/mod.rs:1287, 1337, 1978, 2290, 2305`; `local_agent_invocation.rs:1344` (inside `#[cfg(test)] fn map_agent_invocation_outcome_ok_maps_result` at `:1342`). `AgentResult` derives only `Debug` — no `PartialEq`/serde to satisfy.

**New `AgentErrorCode` variant:**
- `nomifun-api-types/src/agent_error.rs:14-54` — additive.
- `is_provider_fault` (`nomifun-conversation/src/model_failover.rs:62-83`) is a `matches!` list → the new variant is correctly *not* a provider fault with **no code change**; its test at `:546-555` stays green. (Failover would not fire anyway: the seam is pre-response only, and this verdict follows streamed text.)
- **No exhaustive `match` on `AgentErrorCode` exists in the repo** — the `match code` hits at `nomi-a11y/src/windows/tree_map.rs:140,151`, `nomifun-channel/src/plugins/qqbot/gateway.rs:156` and `nomifun-mcp/src/routes.rs:185` are unrelated enums.
- `agent_error_code_token` (`relay_error_code.rs:164-172`) is serde-driven → the token comes free.
- **Not a ts-rs export.** `grep AgentErrorCode ui/src` returns a single doc comment. `ui/src/renderer/pages/companion/companionError.ts:26-56` has a `default:` fallback; `ui/src/common/chat/chatLib.ts:679-737` validates *ownership/resolution* sets, not codes. No UI edit required.
- **No DB migration.** `017_conversation_receipt_error_codes.sql:13-14` is `result_error_code TEXT CHECK (result_error_code IS NULL OR trim(result_error_code) <> '')` — no enumerated CHECK. The `012`/`017` immutability triggers only forbid *changing* a settled value, and this is written once.

**New `anthropic_shared::StreamState` field:** only constructor `StreamState::new()` (`:279-297`); `Default` delegates (`:272-276`); no struct literal anywhere in `nomi-providers` (`bedrock.rs:381` and `:392` call `::new()`).

**Deliberately avoided, and therefore verified unaffected:**
- *No new `AgentEngine` field.* The 7 exhaustive literals that would have broken: `engine/mod.rs:639`, `:718`, `compact_tests.rs:133`, `handle_command_tests.rs:57`, `phase6_tests.rs:60`, `plan_mode_tests.rs:64`, `set_config_tests.rs:1119` (`make_engine`, `:1085-1125`, no `..Default::default()`). No shared builder exists, and `resume_with_provider:718` sets `context_contributors: Vec::new()` — which is why the contributor approach was rejected.
- *No `Session` change.* `Session` literals at `session.rs:128`, `engine_test.rs:1134`, `deferred_activation_test.rs:150`, `factory/nomi.rs:1780`, `:1840` untouched; `host_context` stays `BTreeMap<String, String>`.
- *No `StopReason` variant.* The 3 exhaustive matches that would have broken: `engine/mod.rs:424-429` (`terminal_dimensions`), `manager/nomi/agent.rs:776-780` (`map_engine_stop_reason`, pinned by a unit test), `local_agent_invocation.rs:560-570`. Plus `openai.rs:1755-1780`'s `unreachable!("finish reasons are normalized above")`.
- *No `TurnStopReason` variant* → no ts-rs churn on `protocol/events/mod.rs:125-140`, and A1's `incomplete_stop_code` keeps its exhaustive 5-arm match.
- *No `TokenUsage` field.* Would have broken 30+ exhaustive literals (`nomi-types/src/llm.rs:87`; `openai.rs:740`; `gemini.rs:787`; `anthropic_shared.rs:741`; `tests/common/mod.rs:39,63`; ~20 in `engine_compact_test.rs`/`engine_test.rs`; `autocompact_test.rs:47`; `json_stream_approval_test.rs:23`) and changed the persisted session shape + JSON schema (`Serialize + JsonSchema`). C1.

**Existing tests that must be edited (not deleted):**
- `backend_output_sink.rs:3058-3086` `auto_continue_marks_active_tool_as_truncated_not_completed` → delete with the function.
- `backend_output_sink.rs:3087-3098` `auto_continue_ignores_finished_tool` → delete with the function.
- `backend_output_sink.rs:3099-3131` `fail_active_tool_calls_marks_pending_tool_error_and_drains_it` → keep; drop the two trailing lines that call the removed method.
- `backend_output_sink.rs:4781-4784` inside `update_plan_result_emits_plan_event` → drop the 3 lines calling the removed method.
- `backend_output_sink.rs:4864-4866` (the plan-context test) → same.
- `anthropic_shared.rs:1543-1573` `anthropic_max_tokens_discards_even_a_complete_staged_tool_call` → still asserts no `ToolUse` and a `MaxTokens` `Done`, but now expects **2** events (one `ToolUseTruncated` + `Done`). Rename to `..._reports_it_as_truncated`.
- `local_agent_invocation.rs:1341-1360` `map_agent_invocation_outcome_ok_maps_result` → add the two new `AgentResult` fields.

---

## 10. What is DELETED

| Location | Deleted |
|---|---|
| `manager/nomi/agent.rs:715-718` | `const MAX_TRUNCATION_AUTO_CONTINUES: usize = 2;` and its doc |
| `manager/nomi/agent.rs:720-733` | `fn truncation_continuation_prompt(..)` — the static English recovery prose, in full. (It has **no** unit tests: repo-wide grep = definition + one call site.) |
| `manager/nomi/agent.rs:2049-2086` | the entire host `MaxTokens` continuation branch: `truncation_auto_continues`, the `truncate_active_tool_calls_for_auto_continue` call, `run_content = vec![prompt]`, the `continue`, and the `warn!`-then-clean-`Finish` fall-through |
| `manager/nomi/agent.rs:2038` | `let mut truncation_auto_continues = 0usize;` |
| `capability/backend_output_sink.rs:2262-2273` | `fn truncate_active_tool_calls_for_auto_continue` + its 2 dedicated tests (`:3058-3098`) |
| `openai.rs:1773` | `state.tool_calls.clear();` (the `"length"` arm) — the accumulators now survive to the single drain point |
| `openai.rs:719` | `self.tool_calls.clear();` → `drain_truncated_tool_calls` |
| `anthropic_shared.rs:781` | the silent `state.pending_tool_calls.clear();` in the `"max_tokens"` arm |
| `gemini.rs:749-753` | the `ProviderError::Parse("Gemini stopped at MAX_TOKENS with an uncommitted function call")` arm |

No compatibility shim, no config flag restoring append-prose, no dual write of the ledger, no second retry loop at the host altitude.

---

## 11. Test plan

`crates/agent/nomi-providers` — `cargo test -p nomi-providers openai:: anthropic_shared:: gemini::`
- `length_finish_mid_write_reports_a_truncated_tool_call`: `finish_reason: "length"` mid-`Write` → exactly `[ToolUseTruncated{ name:"Write", argument_bytes: N }, Done{MaxTokens}]`, **no** `ToolUse`.
- `a_post_finish_argument_fragment_is_counted_once`: fragment after the first `finish_reason` → **one** event whose `argument_bytes` is the full final length (pins the deferral invariant at `:1746-1750`).
- `anthropic_max_tokens_reports_it_as_truncated` (edit of `:1543`).
- gemini `MAX_TOKENS` with a pending call → `ToolUseTruncated + Done{MaxTokens}`, not `Err`.

`crates/agent/nomi-agent` — `cargo test -p nomi-agent --lib round:: engine::set_config_tests::`
- `round.rs` units: digest stability over `Text`+`Image`; effect bound at 24 oldest-dropped; `has_open_plan`; unparsable JSON → `None`; `render_section() == None` with an empty ledger.
- `a_round_restart_keeps_host_context_and_rewind_destroys_it` — the two assertions side by side, mirroring `:1389-1421`.
- `a_round_restart_pops_only_the_assistant_draft_and_keeps_every_user_message` — with a drained steer already in the transcript.

`cargo test -p nomi-agent --test badcase_regression_test` — three scripted cases over the existing `scripted_server` (`:127-140`) + `RecordingResponder` (`:106-125`), `session: SessionConfig { enabled: true }` for the durability case:
1. **the observed shape** — prose + `finish_reason:"length"`, no tools called: assert **exactly 1** provider request (no auto-restart), `stop_reason == MaxTokens`, and the assistant text preserved in `result.text`. Pins Blocker 6.
2. **the recoverable shape** — pass 1 `update_plan` + `Bash`; pass 2 prose + `"length"` mid-`Write`: request 3's body must contain **zero** bytes of pass-2 prose, contain the original requirement text **exactly once**, contain `ORIGINAL` round facts and `6142` in the `system` field and in **no** message, and the last message must be `role: user`. Then truncate again → cap → assert exactly 3 requests (**not** `3 × max_turns`) and `rounds == 3`.
3. **compaction between rounds** — `CompactConfig` threshold forced low so `run_compaction` fires on the pass after the restart: assert the restart still happens with `editable_turn == None`, and request N is `[boundary, summary, requirement]`.
4. **the lie** — restart, then a 6-token `"Created miniapp.html."` with `finish_reason:"stop"`: assert `rounds == 2 && effects_ok == 0`.

`crates/agent/nomi-agent` — `cargo test -p nomi-agent --lib session::tests::host_context_survives_session_refresh` extended with a `nomi.round.ledger` round-trip, plus a new test that `resume_with_provider` + a matching digest adopts the ledger and resets `attempt` to 1.

`crates/backend/nomifun-ai-agent` — `cargo test -p nomifun-ai-agent --lib manager::nomi::agent:: capability::backend_output_sink::`
- `rounds_gt_one_with_no_effects_emits_error_not_finish` (scripted provider, `ScriptedProvider` precedent at `agent.rs:4535`), asserting the durable text row survives.
- the 3 edited `backend_output_sink` tests compile and pass.

`crates/backend/nomifun-api-types` — `cargo test -p nomifun-api-types` (serde name → `USER_LLM_PROVIDER_NO_PROGRESS_AFTER_RESTART`).

`crates/backend/nomifun-conversation` — `cargo test -p nomifun-conversation --lib relay_error_code::` (A1's suite, unchanged by B1 — proves B1 adds no second adjudicator).

`cargo clippy -p nomi-types -p nomi-providers -p nomi-agent -p nomifun-api-types -p nomifun-ai-agent --all-targets`.

No UI or ts-rs change → **no `bun` command is required**. (Independently, `bun run test:ui` has a known pre-existing failure from upstream's `CreateStudio` modal restyling.) `nomifun-app`'s full lib suite is not run: it is untouched and has a known one-rotating-test loopback flake.

---

## 12. How this design behaves on the observed failure

Provider `openai-compatible`, model `step-3.7-flash`, ceiling 8192, `output_tokens = 24576 = 3 × 8192`, zero tool calls, `result_ok = 1`, nothing on disk.

1. **Turn 1, pass 1.** `openai.rs` streams text, then `finish_reason: "length"` → normalized `"length"` (`:1409-1412`) → `pending_done = Done{MaxTokens}` (`:1768-1778`). `state.tool_calls` is empty (no call was ever started), so `drain_truncated_tool_calls` yields nothing. Engine: `stop_reason = MaxTokens`, `tool_calls.is_empty()`, assistant message pushed at `:1906`, `*safe_messages` set at `:1915`.
2. **Restart predicate.** `tools_advertised = true`, but `truncated_calls` is empty, no `update_plan` was called, `effects_total == 0` → **`false`**. No restart. The engine returns `Ok(AgentResult { text: <the prose>, stop_reason: MaxTokens, rounds: 1, effects_ok: 0, .. })`. **The 2nd and 3rd 8192-token passes never happen.** `output_tokens` drops from 24576 to 8192 — a 67% cost reduction on this exact failure, because the two passes that were spent re-generating prose after an English "[Automatic continuation]" prompt are gone.
3. **Host.** The deleted branch no longer appends `truncation_continuation_prompt` and no longer swallows the outcome. `rounds > 1` is false, so B1's verdict branch does not fire. `map_engine_stop_reason(MaxTokens) → TurnStopReason::MaxTokens` (`:776-780`) → `commit_verified_turn_if_not_cancelled(.., MaxTokens)` → `emit_finish_for_turn(turn, None, Some(MaxTokens))` (`agent.rs:2516`).
4. **Receipt.** `FinishEventData.stop_reason` → `RelayOutcome.stop_reason` (`stream_relay.rs:3712-3714`) → A1's `incomplete_stop_code(Some(MaxTokens)) = Some(OUTPUT_TRUNCATED)` → `turn_succeeded` is **false** → `result_ok = 0`, `result_error_code = "output_truncated"`, `result_error_retryable = 1`. The knowledge write-back is skipped (`service.rs:9059`). The user's visible prose row is preserved (`stream_relay.rs:3748-3766`). **`result_ok = 1` with nothing on disk is gone.**
5. **Recovery is now the user's, informed.** The receipt carries a retryable, specifically-resumable code that D1 renders as a Continue action; C1 gives the per-capability-row ceiling that makes the retry actually fit.
6. **The counterfactual this workstream exists for.** Had the model called `update_plan` and `mkdir` before running out of room — the far more common shape — the predicate is `true`: the engine pops the 8192-token draft (so it leaves the *provider request* entirely, verified because `messages.pop()` targets the message pushed at `:1906`), re-pushes the original requirement verbatim at the tail, injects `[resumable round 2/3]` + the declared plan + `ok Bash: mkdir -p toolbox` into the **system** prompt, re-sets the rollback floor, and runs pass 2 against the original requirement with a ledger of what is done — instead of asking the model to continue a half-written string. If pass 2 then answers `"Created miniapp.html."` with `EndTurn` and zero effects, `rounds = 2 && effects_ok = 0` emits a terminal `Error` with `user_llm_provider_no_progress_after_restart` — so closing the old false-success path does not open a new one.