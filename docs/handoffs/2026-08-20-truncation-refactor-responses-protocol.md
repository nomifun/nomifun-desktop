# `openai.responses` + `previous_response_id` — corrected design

Verified against `HEAD c603bd7f`. **The line numbers in the task brief are stale**; every anchor below was re-read from this tree. Corrections: Agent protocol allowlist is `manifest.rs:962-969` (not `:885-892`); `allowed_auth_schemes` is `manifest.rs:424-445` with the strict arm at `:429` (not `:355-373`); the provider-params contract test is `manifest.rs:1186-1204` (not `:1106`); the endpoint-template test is `manifest.rs:1294-1367` (not `:1304`); `create_provider` is `nomi-providers/src/lib.rs:603-656` (not `:564-616`); `agent_chat_protocol_contract.rs` pins four families at `:88-170`, and the "exactly four" is a **doc comment at `:1`**, not an assertion.

---

## 0. Scope and limits (read first)

| | |
|---|---|
| Presets that can select `openai.responses` | **1 of 42** — `OpenAI` only. `scopes: NATIVE_ONLY`. Every other preset is rejected **at save time**, not at first invocation (`provider_model.rs:263-273`). |
| Private gateways serving `/responses` (vLLM/LiteLLM) | **Not served.** This is a deliberate regression from the prior draft's `NATIVE_CUSTOM`, which would have listed the protocol in the Chat dropdown for all 42 presets including StepFun — save-then-404. |
| Azure OpenAI | Not served (`api-key:` header; `openai.responses` allows only `bearer`). |
| Does this fix the reported StepFun failure? | **No.** `step-3.7-flash` cannot reach `/responses`. A1/A2 (adjudication), B1 (resumable round) and C1 (ceiling) fix it. This workstream gives a **cheaper and reasoning-preserving continuation primitive on one platform**, and §13 traces what it *would* have done had the provider been OpenAI. |
| Chain lifetime | Bounded by the first **in-place history rewrite** of the already-sent prefix (microcompact, image pruning, image redaction, post-error image strip, resume-time sanitize) and by any **autocompact**. On a long coding session that is *usually a handful of rounds, not the whole session*. This is not a bug to be fixed later; it is forced by the fact that server-retained state is immutable (§3). |
| Chain does not survive a session resume | Correct and deliberate (§4, row `history_sanitize.rs:55`). |
| Truncated round **with** open tool calls cannot chain | Correct and deliberate (§1). The recovery is a full-snapshot round. |
| Reasoning continuity is a double-edged benefit | Retaining server-side reasoning lets a model *resume* an interrupted thought — and equally lets it resume a runaway one. The bound is B1's `MAX_TRUNCATION_AUTO_CONTINUES = 2` (`manager/nomi/agent.rs:718`). No forcing function for a tool call is added here. |
| `store: true` retains the conversation on OpenAI for 30 days | Opt-in per capability row, surfaced in the UI with explicit retention copy (§8 step 16). A stream retry (`retry.rs:171-179`) can leave one orphaned stored response upstream; already billed, never chained to. |
| Cost containment | **No DB migration** (`032_provider_model_capabilities.sql:60` is `protocol TEXT NOT NULL CHECK (trim(protocol) <> '')`, no value allowlist). **No URL-snapshot hash change.** **No `LlmRequest` field.** **No `host_context` key.** **No `LlmEvent::Done` field.** |

---

## 1. The chaining state machine (corrected)

The prior draft armed the chain from the engine and let the engine decide. That is wrong: **only the provider knows whether a round is a legal chain parent**, because only the provider knows (a) whether it actually sent `store:true`, (b) the terminal `response.status`, and (c) whether it dropped tool accumulators the engine will therefore never answer.

So: **the provider decides, and it signals by emitting the id at all.**

### 1.1 Armed

`LlmEvent::ProviderRoundId(id)` is emitted **exactly once, immediately before `Done`, inside the same `drain_terminal_events` call**, and only when **all** of:

1. this attempt was sent with `store: true` (i.e. `compat.chain_rounds()` is on), **and**
2. terminal `response.status` is `"completed"`, or `"incomplete"` with `incomplete_details.reason == "max_output_tokens"`, **and**
3. **every** `function_call` item in the terminal `response.output` was already emitted as a complete `LlmEvent::ToolUse`. Equivalently: no accumulator was dropped.

Condition 3 is what repairs the critique's fatal flaw. `engine/mod.rs:1750-1761` rejects an `EndTurn|MaxTokens` Done carrying tool calls, so on `MaxTokens` the provider **must** discard accumulators. If it discarded any, the parent response holds `function_call` items the host can never answer, and a chained request against it is a hard 400. Condition 3 makes that parent **never become a parent**.

Note what still chains: `status:"incomplete"/max_output_tokens` with **zero** tool calls — which is precisely the production failure shape (runaway reasoning/prose, no tool calls). That is the case chaining exists for and it is armed.

### 1.2 Broken

The chain is **not armed** whenever the newest assistant message in `request.messages` carries no round id. That single rule subsumes:

| Cause | Mechanism |
|---|---|
| provider judged the round unchainable (§1.1) | no id emitted → nothing to attach |
| round failed validation after `Done` (`:1745` `done_count != 1`, `:1750-1763` terminal shape, `:1769-1775` `tool_retry_tracker.assign`) | the assistant message is never pushed (`:1906-1907`) → the id is dropped with the local |
| `supersede_written_draft` collapsed this round's own draft (`:1898`) | engine deliberately does not attach (§3) |
| protocol/serializer switched mid-session (`openai.chat_text`, Anthropic, Gemini) | those providers emit no `ProviderRoundId`, so the newest assistant has none |
| `chain_rounds` turned off | `store:false`, no id emitted |
| in-place rewrite of the already-sent prefix | explicit `break_provider_round_chain()` (§4) |
| transcript replaced/truncated/cleared | the id is physically gone with the message (§4) |

**Read rule (the whole reader, verbatim):**

```rust
// crates/agent/nomi-providers/src/openai_responses.rs
/// Chain only from the NEWEST assistant message. An intervening un-chainable
/// round means the server-side prefix no longer ends where the transcript does,
/// so an older id is not a legal parent even though it still resolves upstream.
fn chain_parent(messages: &[Message]) -> Option<(usize, &str)> {
    let (index, message) = messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, message)| message.role == Role::Assistant)?;
    let id = message.provider_round_id.as_deref()?;
    (index + 1 < messages.len()).then_some((index + 1, id))   // delta must be non-empty
}
```

`(index + 1, id)` is `(delta_from, previous_response_id)`. `delta_from <= messages.len()` is a type-level fact, not a check: it is derived from an index into the same slice. **No panic and no drift are representable.**

### 1.3 Recovery, in a place structurally capable of it

Two failures, two recoveries. Both live in the **provider**, which is the only layer holding both the full transcript and the ability to re-POST.

**(a) `400 … previous response with id 'resp_…' not found`** (expired, deleted, wrong account, stale id that survived a CLI resume).

Handled by a bounded negotiation loop in `OpenAIResponsesProvider::stream`, **structurally identical to the existing two-extension loop at `openai.rs:799-849`**:

```rust
// nomi-providers/src/lib.rs — new classifier beside is_tool_schema_incompatible (:149)
// and is_stream_usage_options_incompatible (:166). Same pattern, same file.
impl ProviderError {
    /// The stored parent named by `previous_response_id` no longer resolves.
    /// A full-snapshot round is self-sufficient, so this is recoverable exactly
    /// once per request without a legacy fallback: the wire format is unchanged.
    pub(crate) fn is_stale_previous_response(&self) -> bool;   // Api{status:400|404, message}
}
```

```rust
let mut chain = Self::chain_parent(&request.messages);
let (response, headers, body) = loop {
    let body = self.build_request_body(request, sanitize_tool_schemas, chain);
    match self.send_initial_with_key_rotation(&client, &url, &body).await {
        Ok(pair) => break (pair.0, pair.1, body),
        Err(error) if chain.is_some() && error.is_stale_previous_response() => {
            tracing::warn!(target: "nomi_providers", provider = "openai.responses",
                parent = %chain.unwrap().1,
                "stored parent response no longer resolves; resending the full transcript");
            chain = None;                    // one-shot, cannot loop
        }
        Err(error) => return Err(error),
    }
};
```

Why this is complete, and why the prior draft's version was not:
* **No durable clear is needed.** The reader only ever consults the *newest* assistant message. The recovered round's own assistant message gets a fresh id, permanently shadowing the stale one. This is the concrete payoff of the representation change in §4.
* **`store` is not coupled to mode** (§2), so the recovered full-snapshot attempt is still `store:true` → it still yields a usable id → the chain **re-arms correctly**. The prior draft's `store:false` Mode S produced an unstorable id and re-armed with garbage every turn.
* Bounded by construction: `chain` can only transition `Some → None`.

**(b) HTTP 404 on `/responses`** (endpoint override points somewhere that does not serve it). Non-retryable `ProviderError::Api { status: 404, .. }` with text naming the fix: *"this connection does not serve /responses; select protocol openai.chat_text for this model, or correct the endpoint override."* **No fallback to `/chat/completions`** (CONTRIBUTING no-legacy-fallback; falling back would hide the misconfiguration behind a working-but-wrong request). The HTML-gateway case is already covered by `ProviderError::NonApiResponse` (`lib.rs:84-89`), reported non-retryable.

---

## 2. `store` and `previous_response_id` — exactly what decides each

The prior draft could never bootstrap because it derived `store` from `provider_round.is_some()`, giving `store:false` on round 1 — and `store:false` + `previous_response_id` is rejected upstream, so every round was round 1.

**Corrected: the two are independent.**

| field | decided by | round 1 of a chained session |
|---|---|---|
| `store` | `compat.chain_rounds()` **only**. Written on **every** request, never implicit (OpenAI's own default is `true`; omitting it silently retains for 30 days). | `store: true` |
| `previous_response_id` | present iff `chain_parent(&request.messages)` is `Some` (§1.2) | **absent** |
| `input` | `Some` → items from `messages[delta_from..]`. `None` → full snapshot of all messages. | full snapshot |

Bootstrap, stated as an invariant: **round 1 sends `store:true` with no `previous_response_id` and a full `input`.** Round 2 finds round 1's id on the assistant message and sends a delta. This is a pinned test (§12).

`store:false` + `previous_response_id` is unrepresentable: `chain_parent` can only return `Some` if a previous round emitted an id, and a round only emits an id if it was sent with `store:true` (§1.1 condition 1).

---

## 3. Reconciling server-retained state with client-side history rewriting

**The conflict, precisely.** `engine/mod.rs:1898` calls `supersede_written_draft(&mut assistant_content)`, then `:1906-1907` pushes the *collapsed* copy. Its own doc at `:1892-1896`: *"Only the copy that enters durable history is collapsed."* In a chained round the parent response on OpenAI's servers holds the **uncollapsed** runaway draft, so the model would re-read it forever — the exact pathology being fixed. Server state is immutable; it cannot be rewritten to match. Same class: `redact_user_images_since` (`:2381`, invoked `:1198` and `:2704`), `strip_tool_images_after_provider_error` (`:2359`, invoked `:1195`), `prune_old_tool_images` (`:2324`, invoked `:1348` and `:2279`), and `micro::microcompact` → `clear_old_tool_results` (`micro.rs:107`, invoked `engine/mod.rs:2421`).

**Chaining does not disable any of them.** The reconciliation rule is:

> **Chain invariant.** A message may carry `provider_round_id` only while the exact content of every message at or before it, as the host would serialize it *now*, equals what was sent in the request that produced it. Any host rewrite that violates that drops the id — the rewrite always wins, the chain always yields.

Concretely, split by *when* the divergence is created:

**(i) Divergence created at commit time — cannot be detected later, must be refused up front.**
`supersede_written_draft` returns `bool` at `:1898`. If it returns `true`, the engine **does not attach the id**:

```rust
// engine/mod.rs, replacing :1898-1907
let superseded = supersede_written_draft(&mut assistant_content);
if superseded {
    tracing::debug!(target: "nomi_agent", turn = turn + 1,
        "collapsed a pre-tool draft superseded by a file write in the same round");
}
let mut assistant = Message::now(Role::Assistant, assistant_content);
// A collapsed draft exists only in local history; any server-retained copy of
// this round still holds the uncollapsed prose. Refusing the parent pointer is
// what keeps the collapse effective instead of silently defeating it.
assistant.provider_round_id = provider_round_id.filter(|_| !superseded);
self.messages.push(assistant);
```

**(ii) Divergence created by a later rewrite of an already-sent message — detected at the rewrite site.**
Each of the four in-place rewriters calls one helper (§4 table). The rewrite is unaffected; only the chain breaks, and the next round is a full snapshot **of the rewritten transcript** — so the token savings and the privacy redaction both land on the wire, which is exactly what they exist for.

**(iii) Divergence by wholesale replacement — free.** Autocompact (`:2461`) sets `self.messages = result.messages`, and `auto.rs:211` builds `vec![boundary_msg, summary_msg]` — two fresh `Message`s. No id survives. The turn-error rollback (`:1188`) restores `safe_messages`, a clone taken at `:1160`/`:1915`/`:2282` — i.e. *before* the id was attached. No id survives.

Honest consequence, restated for §0: **Mode C's lifetime is bounded by the first prefix rewrite.** Microcompact is the tightest bound: its savings are worthless while a chain is live (the immutable server prefix still holds the full tool results), so breaking the chain there is not merely safe, it is the only way microcompact does anything at all.

---

## 4. Desync: derivable, not tracked

The critique's third fatal flaw was a parallel in-memory cursor (`chained_prefix_len`) that drifts from `self.messages` across the in-turn rollback at `engine/mod.rs:1188`, which restores `messages` but not `host_context`, then persists at `:1197`/`:2563`.

**The corrected design has no cursor, no `host_context` key, and no fingerprint. The parent pointer is a field of the message it describes.**

```rust
// crates/agent/nomi-types/src/message.rs — Message (:64-72)
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
    /// Opaque provider-side identity of the round that produced this assistant
    /// message, usable as the parent of the next round by protocols that retain
    /// conversation state server-side (`openai.responses`). Model-invisible
    /// host bookkeeping, exactly like `ToolUse.extra` (:23-26) and
    /// `Thinking.signature` (:46-48). Absent for every protocol that carries the
    /// complete conversation on each request — which is all of them but one, so
    /// this is an optional field, not a compatibility shim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_round_id: Option<String>,
}

/// Drop every server-side parent pointer in `messages`. Called by any host
/// rewrite of a message that has already been sent: retained provider state is
/// immutable, so the local edit and the chain cannot both be honored.
pub fn clear_provider_round_ids(messages: &mut [Message]) -> bool;
```

`Message` has **exactly two** literal construction sites in the whole tree (`compact/micro.rs:232`, `tests/microcompact_test.rs:48`); everything else goes through `Message::new`/`Message::now`, which set the field to `None`. `Message` has no `ts-rs` export (`nomi-types/Cargo.toml` has no `ts-rs` dependency), no `JsonSchema`, and reaches disk only through `Session.messages` (`session.rs:35`) as JSON. No DB CHECK constraint touches it.

### 4.1 Every lifecycle site, named and closed

| # | Site | What it does | Chain effect | Edit |
|---|---|---|---|---|
| 1 | `engine/mod.rs:1188` `self.messages = safe_messages` (turn-error rollback, persisted at `:1197`) | restores the pre-turn clone from `:1160`/`:1915`/`:2282` | **free** — clone predates the attach | none |
| 2 | `engine/mod.rs:1195` `strip_tool_images_after_provider_error()` | clears images in already-sent tool results | breaks | `clear_provider_round_ids` inside `:2359` |
| 3 | `engine/mod.rs:1198` / `:2704` `redact_user_images_since()` | replaces sent user images with markers; already returns `bool` | breaks | `clear_provider_round_ids` inside `:2381` |
| 4 | `engine/mod.rs:1348`, `:2279` `prune_old_tool_images()` | drops images from possibly-sent tool results | breaks | make `:2324` return `bool`; call `clear_provider_round_ids` when it removed anything |
| 5 | `engine/mod.rs:2421` `micro::microcompact()` (`result.cleared_count > 0` at `:2422`) | overwrites old tool-result content | breaks | `clear_provider_round_ids(&mut self.messages)` in the existing `> 0` branch |
| 6 | `engine/mod.rs:2461` autocompact success | `self.messages = result.messages` (`auto.rs:211`, two fresh messages) | **free** | none |
| 7 | `engine/mod.rs:2606` `clear_context()` `self.messages.clear()` | empties | **free** | none |
| 8 | `engine/mod.rs:2645` `rewind_last_turn()` `truncate(start_len)` | truncates | **free** | none |
| 9 | `engine/mod.rs:815` `init_session()` | clears `host_context` only, not messages | **free and correct** — ids still describe the messages they are on | none |
| 10 | `engine/mod.rs:2697` `abort_current_turn()` | **appends** synthetic tool results | **free** — append-only, prefix untouched; the parent's calls are answered | none (the `redact_user_images_since(0)` at `:2704` is row 3) |
| 11 | `history_sanitize.rs:55` `sanitize_session_messages()` | resume-time in-place content repair; called on both halves and both early returns via `factory/nomi.rs:98,102,106,107` | breaks | one unconditional `clear_provider_round_ids` at the top |
| 12 | `factory/nomi.rs:67` `retarget_resumed_session()` | model/provider retarget; **return value discarded at `:527`** | **free** — row 11 runs first at `:519` on the same path | none |
| 13 | `engine/mod.rs:1906-1907` assistant push | the **only** writer | attach per §3(i) | the §3 snippet |

Rows 1, 6, 7, 8, 9, 10, 12 are free **because the cursor is the message**. That is the whole argument: the prior draft needed all seven of them enumerated *and correct*; here they cannot be wrong.

There is exactly **one** writer (row 13) and exactly **one** reader (`chain_parent`, §1.2). No torn window, no double save — the id rides the same `save_session()` that persists the message (`:2560` `session.messages = self.messages.clone()`).

---

## 5. Corrected scoping

```rust
// crates/backend/nomifun-model-invoke/src/manifest.rs — insert after :465
ProtocolSpec { id: "openai.responses", tasks: &[Chat], executor: Agent, transport: Http,
    scopes: NATIVE_ONLY, platforms: &["openai"], connection_role: None,
    endpoints: &[endpoint(Chat, "endpoint", Submit, "POST", "/responses")] },
```

`NATIVE_ONLY` (`manifest.rs:415`), not `NATIVE_CUSTOM`. Consequences, all verified:

* `protocol_manifest_for_connection` admits a descriptor iff its `platforms` contains the selected platform **or** its `scopes` contain `Custom` (`manifest.rs:1045-1048`). With `NATIVE_ONLY` the protocol appears only for `platform == "openai"`, so `ModelDefinitionEditor.tsx:610` (`[...(manifest?.protocols ?? [])]`, no client-side filter) never renders it for StepFun.
* Not merely hidden — **rejected at save time**, in a check the prior draft did not know existed: `provider_model.rs:263-273` computes `supports_platform = descriptor.platforms.contains(platform) || scopes.contains(Custom)` and returns `"protocol {protocol:?} is not available for provider platform {platform:?}"`. A hand-crafted `POST /api/providers` with `openai.responses` on a StepFun row is a 400. `bedrock.anthropic_messages` (`manifest.rs:467`) is the in-tree precedent for a `NATIVE_ONLY` Agent Chat protocol.
* **Zero protocol-list assertion churn.** `custom_has_all_configurable_task_protocols_but_no_default` (`manifest.rs:1461-1473`) and `tests/protocol_manifest.rs:143-156` keep their 3-id lists; `manifest.rs:1478` and `tests/protocol_manifest.rs:184` keep `protocols.len() == 3`. No test asserts the OpenAI-preset Chat list (grepped). The prior draft's six assertion edits collapse to zero.

`EndpointRootShape` is forced: `/responses` contains no version segment, so `manifest.rs:141-155` requires `declared_origin_root == false`, i.e. `endpoint()` (`:358-374`, `root: VersionedRoot`), not `origin_endpoint()`. The OpenAI preset base is `https://api.openai.com/v1` (`manifest.rs:249`), so the shipped-default-connection shape check at `:165-174` passes. `url_algebra::join_endpoint` needs no change.

---

## 6. The existing footgun (fixed here)

Today `endpoint` is `editable: true` (`manifest.rs:371`, enforced by `every_non_sdk_endpoint_is_user_editable` at `:1407-1422`) and `validate_endpoint_template` (`:604-656`) checks only blankness and placeholder vocabulary. A user can type `/responses` into the `openai.chat_text` Chat row and get a `chat/completions` body POSTed there → opaque 400 with nothing naming the real mistake. Adding `openai.responses` makes the mirror image possible.

Fix in the manifest, so both authorities inherit it with zero new plumbing — the only two callers of `validate_endpoint_template` are `provider_model.rs:337` (save time, inside `validate_endpoint_overrides`) and `resolve.rs:219`/`:237` (runtime, for pre-contract/corrupt rows):

```rust
/// Endpoint paths that belong to a DIFFERENT protocol's wire format. The final
/// path segment is the only discriminator these two APIs give us, so the
/// manifest owns the mapping. Deliberately only these two: a path ending in
/// `messages` is NOT listed, because a compatible gateway may legitimately
/// serve chat/completions there and this table must never cost a working
/// configuration.
const CROSS_PROTOCOL_ENDPOINT_PATHS: &[(&str, &str)] = &[
    ("/chat/completions", "openai.chat_text"),
    ("/responses",        "openai.responses"),
];

/// The protocol that owns `value`, when that is not `protocol_id`. Scoped to
/// the two OpenAI Chat protocols so no other protocol's endpoint freedom is
/// reduced.
fn conflicting_endpoint_owner(protocol_id: &str, value: &str) -> Option<&'static str> {
    if !matches!(protocol_id, "openai.chat_text" | "openai.responses") {
        return None;
    }
    let path = value.split(['?', '#']).next().unwrap_or_default()
        .trim_end_matches('/').to_ascii_lowercase();
    CROSS_PROTOCOL_ENDPOINT_PATHS.iter()
        .find(|(suffix, owner)| *owner != protocol_id && path.ends_with(suffix))
        .map(|(_, owner)| *owner)
}
```

Wired into `validate_endpoint_template` **after** the field lookup at `:620-628` and before `collect_endpoint_placeholders` at `:629`, so "unknown protocol" and "no such field" still win. Message:

> `protocol "openai.chat_text" endpoint field "endpoint" points at /responses, which is served by protocol "openai.responses"; select that protocol instead of overriding this one`

Fixture safety verified by opening each: `agent_chat_protocol_contract.rs:95-99` → `/custom/chat?api-version=2026-08-11` (no match); `provider_config.rs:583` → `/custom/chat` (no match); `provider_config.rs:609` → `/chat/completions` on `openai.chat_text` (its own owner, allowed); `mimo.chat_asr`/`mimo.chat_tts` (`manifest.rs:504-505`) default to `/chat/completions` but are out of scope by protocol id. No test loops `validate_endpoint_template` over every default template (grepped: the eight in-module calls are all in `endpoint_template_validation_is_the_protocol_placeholder_authority`, `:1294-1367`).

---

## 7. The provider module

### 7.1 New module, plus a small verbatim move

`openai.rs` is 4063 lines. The reusable fraction is envelope-independent policy; the envelopes and the stream parser share nothing (`StreamState` at `:642-786` is built entirely around chat/completions pathologies: sparse `tool_calls[index]`, `finish_reason` echo tolerance, `pending_done` deferral for a trailing usage-only `choices:[]` frame, `[DONE]`-only gateways). `anthropic_shared.rs` is the in-tree precedent for exactly this split.

`crates/agent/nomi-providers/src/openai_shared.rs` (new) — **verbatim move, no behaviour change**, re-point `openai.rs`'s call sites, make items `pub(crate)`:

| item | from |
|---|---|
| `joined_text_blocks(&[ContentBlock], &ProviderCompat) -> String` | `openai.rs:480` |
| `strip_patterns_from_text(&str, &ProviderCompat) -> String` | `openai.rs:496` |
| `generate_call_id() -> String` | `openai.rs:329` |
| `tool_image_data_url(&ToolImage) -> String` | extracted from `openai.rs:338-363` |
| `tool_argument_value_progress_preview(&Value) -> Option<Value>` | `openai.rs:1102` |
| `tool_argument_progress_preview(&str) -> Option<Value>` | `openai.rs:1123` |
| `TOOL_PROGRESS_PREVIEW_FIELDS: &[&str]` | `openai.rs:1083` (**not** `:1076`) |
| `provider_error_detail(&Value) -> String` | `openai.rs:1391` — the prior draft used this in `terminal_stop` but omitted it from the move list |

Shared because they are **user-visible policy** (strip patterns, image encoding, which argument fields appear in the tool-progress chip); two copies would drift and the drift would be visible in the UI. **Not moved:** `MAX_STRUCTURED_TOOL_CALLS_PER_TURN` (`openai.rs:19`, **not** `:139`) — the engine already enforces the identical bound independently at `engine/mod.rs:70` / `:1545-1550`.

### 7.2 `crates/agent/nomi-providers/src/openai_responses.rs` (new)

```rust
//! OpenAI Responses API (`POST /responses`) serializer and typed-SSE reader.

pub struct OpenAIResponsesProvider {
    api_keys: Vec<String>,
    current_api_key: AtomicUsize,
    base_url: String,
    compat: ProviderCompat,
    sanitize_tool_schemas: AtomicBool,
}

impl OpenAIResponsesProvider {
    pub fn new(api_key: &str, base_url: &str, compat: ProviderCompat) -> Self;
    fn should_sanitize_tool_schemas(&self) -> bool;
    fn build_headers(api_key: &str) -> Result<HeaderMap, ProviderError>;   // Bearer, per manifest:429

    /// Chain only from the newest assistant message (§1.2).
    fn chain_parent(messages: &[Message]) -> Option<(usize, &str)>;

    /// `input` items. `chain = Some((delta_from, _))` emits ONLY
    /// `messages[delta_from..]`; `None` emits a full snapshot.
    fn build_input(
        messages: &[Message],
        chain: Option<(usize, &str)>,
        compat: &ProviderCompat,
    ) -> Vec<Value>;

    /// Flat `{type:"function", name, description, parameters, strict}` items.
    fn build_tools(tools: &[ToolDef], sanitize: bool) -> Vec<Value>;

    fn build_request_body(
        &self,
        request: &LlmRequest,
        sanitize_tool_schemas: bool,
        chain: Option<(usize, &str)>,
    ) -> Value;
}

#[async_trait] impl LlmProvider for OpenAIResponsesProvider { /* stream(), §1.3 loop */ }

/// One accumulating output item, keyed by the provider's own `output_index`.
struct ResponseItemAccumulator {
    output_index: u64, item_id: String, call_id: String, name: String,
    arguments: String, arguments_done: bool, announced: bool,
    last_progress_signature: String,
}

struct ResponsesStreamState {
    response_id: Option<String>,
    items: BTreeMap<u64, ResponseItemAccumulator>,
    usage: TokenUsage,
    reasoning_tokens: u64,       // observed + logged; TokenUsage field is C1's edit
    stored: bool,                // whether THIS attempt sent store:true
    terminal_seen: bool,
    fatal_error: bool,
}

impl ResponsesStreamState {
    fn new(stored: bool) -> Self;
    fn poison(&mut self, message: impl Into<String>) -> Vec<LlmEvent>;
    fn fatal_error(&self) -> bool;
    /// Emit ToolUse* … then at most one ProviderRoundId … then exactly one Done.
    fn drain_terminal_events(&mut self, response: &Value) -> Vec<LlmEvent>;
}

fn parse_responses_event(event_name: &str, data: &str,
                         state: &mut ResponsesStreamState) -> Vec<LlmEvent>;
async fn process_responses_stream(response: reqwest::Response,
                                  tx: &mpsc::Sender<LlmEvent>,
                                  stored: bool) -> StreamOutcome;
```

Reused unchanged: `crate::http_client()` (`lib.rs:577`), `crate::send_initial_with_key_rotation` (`lib.rs:360-368`), `crate::request_body_with_extra` (`lib.rs:46-50`), `crate::parse_api_keys`, `crate::retry::finish_stream_with_retry` (`retry.rs:161`), `crate::anthropic_shared::StreamOutcome` (`:325-329`), `compat::sanitize_json_schema`.

**Not copied:** the `include_stream_usage` negotiation (`openai.rs:799-843`). `/responses` has no `stream_options`; probing there burns a request.

### 7.3 Body

```jsonc
{
  "model": "gpt-…",
  "instructions": "<request.system>",     // NOT a role:"system" input item
  "input": [ /* delta or snapshot */ ],
  "stream": true,
  "store": true,                          // ALWAYS explicit; == compat.chain_rounds()
  "max_output_tokens": 8192,              // name via compat.max_tokens_field
  "tools": [{ "type":"function", "name":…, "description":…, "parameters":…, "strict":false }],
  "reasoning": { "effort": "high" },      // only when request.reasoning_effort.is_some()
  "previous_response_id": "resp_…"        // only when chain_parent() is Some
}
```

`build_request_body` ends exactly like `openai.rs:291-305`: `request_body_with_extra(&self.compat, typed)` then an `as_object_mut()` cleanup block removing `tools` / `reasoning` / `previous_response_id` when absent. **The C1 seam is one line in that block**, pre-commented:

```rust
// C1 (max_tokens: Option<u32>) adds exactly this line here. `request_body_with_extra`
// merges compat.extra_body FIRST (lib.rs:46-50), so a typed None does NOT omit the
// field on the wire — provider_params.max_tokens would silently become the ceiling.
// Omission must also strip it from extra_body, which this removal does.
// if request.max_tokens.is_none() { object.remove(max_tokens_field); }
```

**Mode gating of the compat filters (repairs the contradiction the critiques found).** In a snapshot, a `function_call_output` whose `call_id` has no sibling `function_call` in the same `input` array is a 400, so the orphan/dedup filters are required. In a delta, the matching `function_call` is *by construction* in the parent response, never in `input` — so the same filters would delete **every item in the delta**. Rule, stated once and enforced in one place:

```rust
// build_input
let apply_filters = chain.is_none();   // NOT compat-gated in Mode C
// `/responses` resolves call_id against the stored parent, so a chained delta of
// bare function_call_output items is correct and must pass through untouched.
```

`openai_responses_defaults()` therefore keeps `clean_orphan_tool_calls: Some(true)` and `dedup_tool_results: Some(true)` (correct for snapshots) and `build_input` ignores both when `chain.is_some()`. Pinned by a test (§12).

### 7.4 SSE mapping

`/responses` sends **named** events (`event: response.output_text.delta` + `data: {…}`). `openai.rs:940` reads only `data:` lines and treats `[DONE]` as terminal — neither applies.

| Responses event | read | emitted |
|---|---|---|
| `response.created` | `response.id` | none — captured into `state.response_id` |
| `response.in_progress`, `response.queued` | — | none |
| `response.output_item.added` | `output_index`, `item.{type,id,call_id,name}` | `function_call` → open accumulator; `reasoning`/`message` → none |
| `response.output_text.delta` | `delta` | `TextDelta` |
| `response.output_text.done` | — | none |
| `response.reasoning_summary_text.delta` / `response.reasoning_text.delta` | `delta` | `ThinkingDelta` |
| `response.function_call_arguments.delta` | `output_index`, `delta` | append; `ToolUseDelta` via shared `tool_argument_progress_preview` |
| `response.function_call_arguments.done` | `output_index`, `arguments` | mismatch with accumulator → **poison**; else mark done |
| `response.output_item.done` | `item` | `item.{call_id,name,arguments}` authoritative; mismatch → **poison** |
| `response.refusal.delta` / refusal content part | — | **poison → `Error`** |
| `response.completed` / `response.incomplete` | `response` object | terminal — decide from the object |
| `response.failed` | `response.error` | `Error(provider_error_detail(..))` |
| `error` | `message` | `Error` |
| any other `response.*` **before** terminal | — | ignored (forward-compat) |
| any other name, or **any** event after terminal | — | **poison** |

Terminal decision — the event *name* is a hint, the **object is the authority** (gateways emit `response.completed` carrying `status:"incomplete"`):

```rust
fn terminal_stop(response: &Value, has_complete_calls: bool) -> Result<StopReason, String> {
    match response.get("status").and_then(Value::as_str) {
        Some("completed") => Ok(if has_complete_calls { StopReason::ToolUse } else { StopReason::EndTurn }),
        Some("incomplete") => match response.get("incomplete_details")
            .and_then(|d| d.get("reason")).and_then(Value::as_str) {
            Some("max_output_tokens") => Ok(StopReason::MaxTokens),
            Some("content_filter") => Err("the provider stopped this response for content policy".into()),
            Some(other) => Err(format!("provider returned status=incomplete with unsupported reason '{other}'")),
            None => Err("provider returned status=incomplete without incomplete_details.reason".into()),
        },
        Some("failed")    => Err(provider_error_detail(&response["error"])),
        Some("cancelled") => Err("the provider cancelled this response".into()),
        Some(other)       => Err(format!("provider returned unsupported response status '{other}'")),
        None              => Err("provider terminal event carried no response.status".into()),
    }
}
```

Three consequences worth naming:

1. **The `normalize_finish_reason` folding bug is structurally unrepeatable.** `openai.rs:1399-1417` maps `content_filter` → `"stop"` → `EndTurn` → `result_ok=1`. Here `content_filter` is a *sibling reason under `incomplete`*, not a sibling of `completed`, and `StopReason` (`message.rs:104-114`) has only `EndTurn/ToolUse/MaxTokens/MaxTurns` — so this fails **closed** to `LlmEvent::Error`. When A2 adds `StopReason::Refusal`, that one arm becomes `Done{Refusal}`.
2. **`MaxTokens` discards accumulators** (forced by `engine/mod.rs:1750-1761`), but unlike `openai.rs:1743-1752` which clears silently, `response.output_item.added` gives us `item.name`/`call_id` *before* any argument byte, so emit a structured diagnostic naming every dropped call and its accumulated byte count. **And per §1.1 condition 3, no `ProviderRoundId` is emitted for that round.**
3. **Usage names differ:** `usage.input_tokens → input_tokens`, `usage.output_tokens → output_tokens`, `usage.input_tokens_details.cached_tokens → cache_read_tokens`. `usage.output_tokens_details.reasoning_tokens` has no `TokenUsage` field (`message.rs:116-125`) — captured, logged, **not added** (C1 owns it).

`ProviderRoundId` is **not** in this module's replay-unsafe set (`TextDelta | ThinkingDelta | ToolUseDelta | ToolUse`, the analogue of `openai.rs:969-978`); since it is emitted in the same `drain_terminal_events` batch as `Done`, retry (`retry.rs:171-179`, `FailedEmpty` only) cannot see a stream that emitted it. **The prior draft's "last-wins across a retry" rule is deleted, not fixed** — it is unreachable.

---

## 8. Ordered edit list

Compile-order; each phase leaves the tree building.

**Phase 1 — manifest & footgun (no runtime behaviour yet)**

1. `crates/backend/nomifun-model-invoke/src/manifest.rs`
   - `:429` → `"openai.chat_text" | "openai.responses" => &["bearer"],`
   - after `:465` → the `ProtocolSpec` literal from §5
   - `:683` → add `| ("openai.responses", Chat)` to the `Json` arm of `provider_params_encoding`
   - after `:749` (outside the `task == Chat` block, mirroring the `generation_option_keys` rejection at `:745-749`) → reject `chain_rounds` on every `(protocol, task)` except `("openai.responses", Chat)`, and require a boolean there
   - `:600-603` area → `CROSS_PROTOCOL_ENDPOINT_PATHS` + `fn conflicting_endpoint_owner(protocol_id: &str, value: &str) -> Option<&'static str>`; call it in `validate_endpoint_template` between `:628` and `:629`
   - `:962-969` → add `| "openai.responses"` to the `ProtocolExecutorKind::Agent` allowlist ⚠️ **omitting this makes `try_default_protocol_registry` return `InvokeError::config` at `:976-980` and `default_protocol_registry()` panic at `:985-988` — the app does not boot**
2. `crates/backend/nomifun-model-invoke/src/routes_table.rs` — **no production change.** `openai_route(Chat) => route("openai.chat_text")` at `:26` stays. Enforced by the locked URL snapshot (`manifest.rs:1610`, `tests/protocol_manifest.rs`, hash `9_446_312_405_170_401_367`) which iterates `view.recommendation` only.

**Phase 2 — types**

3. `crates/agent/nomi-types/src/message.rs`
   - `Message` (`:64-72`) → append `pub provider_round_id: Option<String>` with `#[serde(default, skip_serializing_if = "Option::is_none")]`
   - `Message::new` (`:76-82`), `Message::now` (`:85-91`) → `provider_round_id: None`
   - new `pub fn clear_provider_round_ids(messages: &mut [Message]) -> bool`
4. `crates/agent/nomi-types/src/llm.rs` — append to `LlmEvent` after `Error(String)` (`:56`):
   ```rust
   /// Opaque provider-side identity for THIS round, usable as the parent of the
   /// next one. Emitted immediately before `Done`, and only for a round the
   /// provider judged a legal chain parent. Mirrors `ThinkingSignature`:
   /// provider bookkeeping the host round-trips and the model never sees.
   ProviderRoundId(String),
   ```
   **`LlmRequest` (`:8-18`) is not touched.**
5. `crates/agent/nomi-agent/src/compact/micro.rs:232` and `crates/agent/nomi-agent/tests/microcompact_test.rs:48` — the only two `Message` struct literals in the tree: add `provider_round_id: None`.

**Phase 3 — config**

6. `crates/agent/nomi-config/src/compat.rs`
   - after `:81` → `pub chain_rounds: Option<bool>` (last field, minimal merge surface)
   - `merge` (`:145-170`, the **only** exhaustive `ProviderCompat` literal in the tree — every other one at `compat.rs:661/964/994/1004`, `config.rs:2136/2144`, `openai.rs:3134/3375`, `anthropic_shared.rs:899/1066/1083/1098/1114/1133/1154`, `gemini.rs:1046`, `set_config_tests.rs:2705/2762`, `provider_anthropic_test.rs:815`, `provider_openai_test.rs:1329` uses `..Default::default()` or `..*_defaults()`) → one line
   - after `:226` → `pub fn chain_rounds(&self) -> bool { self.chain_rounds.unwrap_or(false) }`
   - after `:124` → `pub fn openai_responses_defaults() -> Self`: `max_tokens_field: Some("max_output_tokens")`, `api_path: Some("/v1/responses")` (**CLI-only; overwritten to `Some("")` on the app path at `provider_config.rs:200` and `agent.rs:966-968` — comment it**), `clean_orphan_tool_calls: Some(true)`, `dedup_tool_results: Some(true)`, `supports_thinking: Some(false)`, `supports_effort: Some(true)`, `effort_levels` as OpenAI, `chain_rounds: None`. **No `merge_assistant_messages`** (items are typed, not role-merged), **no `auto_tool_id`** (`/responses` always supplies `call_id`).
7. `crates/agent/nomi-config/src/config.rs`
   - `:501-508` → `ProviderType::OpenAIResponses`
   - `:512-518` `default_base_url` (exhaustive) → `"https://api.openai.com"` — *CLI-only*
   - `:521-528` `default_model` (exhaustive) → `"gpt-4o"` — *CLI-only*
   - `:532-538` `compat_defaults` (exhaustive) → `openai_responses_defaults()` — **load-bearing on the app path**
   - `:631` `matches!(provider, ProviderType::Anthropic)` for `prompt_caching` — **compiles unchanged; `false` is correct.** (The prior draft's "5 exhaustive matches" list omitted this; recorded so a reviewer does not hunt.)
   - `:667-675` `parse_builtin_provider` → `"openai-responses" => Some(ProviderType::OpenAIResponses)` — **load-bearing**: the app path is `agent.rs:944 provider: Some(config_extra.provider)` → `Config::resolve` → `resolve_provider_alias` (`:698`) → here
   - `:708`, `:718`, `:727` — three error strings enumerating builtins
   - `:763-786` `resolve_api_key` (exhaustive) → fold into the OpenAI arm: `ProviderType::OpenAI | ProviderType::OpenAIResponses => { OPENAI_API_KEY }`
   - `:2276-2282` `test_provider_label_is_builtin_name_for_builtin` — add `("openai-responses", ProviderType::OpenAIResponses)`; compiles without it, but the new arm would ship untested

   No other exhaustive `ProviderType` match exists in the tree (grepped: every use outside `config.rs` and `nomi-providers/src/lib.rs` is a `Config` field init or an `assert_eq!`).

**Phase 4 — provider**

8. `crates/agent/nomi-providers/src/openai_shared.rs` (new) — the §7.1 move. Pure move.
9. `crates/agent/nomi-providers/src/openai_responses.rs` (new) — §7.2-§7.4 + the §1.3 recovery loop.
10. `crates/agent/nomi-providers/src/lib.rs`
    - `:1-7` → `pub mod openai_responses;` `mod openai_shared;`
    - after `:160` → `pub(crate) fn is_stale_previous_response(&self) -> bool` on `ProviderError` (§1.3). **No new `ProviderError` variant**, so the exhaustive `redacted()` match at `:94-123` is untouched.
    - `:603-656` `create_provider` (exhaustive on `ProviderType`) → new arm constructing `OpenAIResponsesProvider::new(&config.api_key, &config.base_url, compat)`
    - `:405-435` `SecretRedactingProvider` — **no change**; `other => other` at `:427` forwards `ProviderRoundId` for free. Add a test that pins it.

**Phase 5 — engine**

11. `crates/agent/nomi-agent/src/engine/mod.rs`
    - after `:1468` → `let mut provider_round_id: Option<String> = None;` (beside `thinking_signature`, the exact structural analogue)
    - after `:1711` (before the `Done` arm at `:1712`) → new arm:
      ```rust
      LlmEvent::ProviderRoundId(id) => {
          if provider_round_id.as_deref().is_some_and(|seen| seen != id) {
              efficiency.observe_calls(&self.tools, &tool_calls);
              return Err(AgentError::ApiError(
                  "provider stream protocol violation: two differing ProviderRoundId events in one round".into(),
              ));
          }
          provider_round_id = Some(id);
      }
      ```
    - `:1898-1907` → the §3(i) attach. **This is the only commit site.** There is no separate "commit after validation" step: the id becomes durable exactly when the message it describes does, so every failure path between `Done` and the push (`:1745`, `:1750-1763`, `:1769-1775`) drops it automatically. Both critiques' conflicting-anchor findings are dissolved rather than patched.
    - `:2324` `prune_old_tool_images(&mut self)` → `-> bool`; call sites `:1348`, `:2279` break the chain on `true`
    - `:2359` `strip_tool_images_after_provider_error` → `clear_provider_round_ids(&mut self.messages)` at the top
    - `:2381` `redact_user_images_since` → same
    - `:2422` `if result.cleared_count > 0` branch → same
    - new `fn break_provider_round_chain(&mut self, reason: &'static str)` wrapping `clear_provider_round_ids` + `tracing::debug!`
    - **No new struct field, no `host_context` key, no `HOST_CONTEXT_*` const.**
12. `crates/backend/nomifun-ai-agent/src/image_generation.rs:185-215` — add `LlmEvent::ProviderRoundId(_) => {}` to the second (and last) exhaustive `LlmEvent` match.

**Phase 6 — resolver**

13. `crates/backend/nomifun-ai-agent/src/manager/nomi/history_sanitize.rs:55` — unconditional `clear_provider_round_ids(messages)` at the top of `sanitize_session_messages`, with the reason in a comment. Covers `factory/nomi.rs:98,102,106,107` (both halves, both early returns) and therefore also `retarget_resumed_session` (`nomi.rs:527`, whose return value is discarded).
14. `crates/backend/nomifun-ai-agent/src/types.rs:96-109` — `pub chain_rounds: Option<bool>` on `NomiCompatOverrides` (plain struct, no ts-rs).
15. `crates/backend/nomifun-ai-agent/src/factory/provider_config.rs`
    - `:102-168` → new `"openai.responses"` arm: bearer check, `provider = "openai-responses"`, `Some(task.http_endpoint()?)`
    - `:166-168` → **update the enumerated error text** to list five protocols
    - `:188-197` area → `provider_body.remove("chain_rounds")` typed extraction. **Load-bearing, not cosmetic:** `:208` is `extra_body: (!provider_body.is_empty()).then_some(provider_body)`, so skipping the removal POSTs `chain_rounds:true` to OpenAI as an unknown body field.
    - `:198-209` → set it on `NomiCompatOverrides`
    - `:266-268` area → `if let Some(chain) = fields.compat_overrides.chain_rounds { config.compat.chain_rounds = Some(chain); }`
16. `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs:970-972` area — the same copy for the long-lived agent path (**this is the app path**).
17. `crates/backend/nomifun-ai-agent/src/services/provider_health.rs:411-419` — **deliberately do NOT copy `chain_rounds`.** A health probe must not create 30-day server-retained state. Comment it and pin it with a test. (This is the third `compat_overrides` copy site; the prior draft listed only two.)

**Phase 7 — UI (the retention notice §0 promises)**

18. `ui/src/renderer/pages/settings/components/providerModelAdvanced.ts` — `providerParamChainRounds(raw: string): boolean` and `withProviderParamChainRounds(raw: string, on: boolean): string`, modelled exactly on `providerParamVoice` (`:511-517`) / `withProviderParamVoice` (`:526-534`).
19. `ui/src/renderer/pages/settings/components/ModelDefinitionEditor.tsx` — a checkbox row rendered only when `capability.protocol === 'openai.responses'`, disabled while `!providerParamsValid`, inserted immediately before the raw provider-params textarea at `:1167`, following the `speech_synthesis`/voice block at `:1120-1165` verbatim. Copy must state that enabling it makes OpenAI retain the conversation for 30 days (`store:true`), same explicitness as the three-state browser visibility policy in `14e3f4ec`/`f6de74b8`. New i18n keys `settings.modelAdvanced.chainRounds{,Hint,Unavailable}` with `defaultValue` fallbacks, matching the surrounding style.

---

## 9. Every gate, and every site that breaks

**Boot / hard-panic gates**

| Path:line | Gate | Failure if omitted |
|---|---|---|
| `manifest.rs:962-969` | Agent protocol-id allowlist | `default_protocol_registry()` **panics** at `:985-988`; app does not boot |
| `manifest.rs:683` | `provider_params_encoding` Json group | `every_manifest_protocol_task_has_a_provider_params_encoding_contract` (`:1186-1204`) **panics** |
| `manifest.rs:464-536` | `PROTOCOL_SPECS` | protocol does not exist |
| `manifest.rs:141-155` / `:165-174` | root-shape + shipped-connection consistency | `try_default_protocol_registry` errors → boot panic (satisfied by `endpoint()`, §5) |

The Agent protocol correctly needs **no** `default_adapters()` entry: the request/realtime consistency loops at `:922-938` and `:940-959` iterate registry ids, not descriptors.

**Silent-misconfiguration gates**

| Path:line | Gate | Failure if omitted |
|---|---|---|
| `manifest.rs:429` | `allowed_auth_schemes` strict-agent arm | falls through to `GENERIC_HTTP_AUTH_SCHEMES` (`:417-422`), advertising `token`/`header_key:<name>`/`query_key:<param>` which `provider_config.rs:104-108` rejects at first invocation — a save-then-fail. **Nothing fails automatically**, which is why item 1 below is added. |
| `manifest.rs:721-809` | `chain_rounds` per-protocol validation | a typo or a wrong-protocol `chain_rounds` silently reaches `extra_body` (`provider_config.rs:208`) |
| `manifest.rs:604-656` | `conflicting_endpoint_owner` | the pre-existing footgun, now bidirectional |
| `provider_model.rs:263-273` | save-time platform/scope gate | **already exists**; `NATIVE_ONLY` activates it for free (§5) |

**Will not compile without an edit**

| Path | Anchor | Why |
|---|---|---|
| `crates/agent/nomi-config/src/config.rs` | `:512-518`, `:521-528`, `:532-538`, `:763-786` | exhaustive `ProviderType` matches |
| `crates/agent/nomi-providers/src/lib.rs` | `:603-656` `create_provider` | exhaustive `ProviderType` match |
| `crates/agent/nomi-agent/src/engine/mod.rs` | `:1534-1741` | exhaustive `LlmEvent` match (1 of 2) |
| `crates/backend/nomifun-ai-agent/src/image_generation.rs` | `:185-215` | exhaustive `LlmEvent` match (2 of 2) |
| `crates/agent/nomi-agent/src/compact/micro.rs` | `:232` | `Message` struct literal (1 of 2) |
| `crates/agent/nomi-agent/tests/microcompact_test.rs` | `:48` | `Message` struct literal (2 of 2) |
| `crates/agent/nomi-config/src/compat.rs` | `:145-170` `merge` | the only exhaustive `ProviderCompat` literal |
| `crates/agent/nomi-agent/src/engine/mod.rs` | `:1348`, `:2279` | `prune_old_tool_images` returns `bool` (statement → `let`/`if`) |

**Exhaustive `LlmEvent` matches: exactly two.** Verified: everything else has a wildcard — `bootstrap.rs:57` and `:122`, `compact/auto.rs:226`, `factory/provider_config.rs:386` and `:431`, `one_shot.rs:145-166`, `openai.rs:397`, `nomi-providers/src/lib.rs:425-428` (`other => other`, so `ProviderRoundId` crosses redaction for free), `gemini.rs:290`/`:882`, `manager/nomi/agent.rs:4901`. `local_agent_invocation.rs`'s `LlmEvent` uses (`:957-1247`, `:1506`) are `#[cfg(test)]` constructors. The TTFT gate (`engine/mod.rs:1520-1527`) and the openai replay-unsafe set (`openai.rs:969-978`) are `matches!`, so the new variant is correctly excluded from both.

**`LlmRequest` literals: 17 across 12 files — and I touch none of them.** (`grep -rn "LlmRequest {" crates/` = 25 hits = 1 declaration at `llm.rs:8` + 7 `fn … -> LlmRequest {` signature lines (`nomi-providers/src/lib.rs:705`, `openai.rs:3180`, `provider_anthropic_test.rs:20` and `:106`, `provider_gemini_test.rs:11`, `provider_openai_test.rs:22` and `:122` — the two `request_with_composed_tool_schema` helpers contain no literal, they mutate another helper's result) + 17 literals. Both the prior draft's "25 sites" and its §9 "25 files" were wrong.)

**Assertion updates**

| Path | Test | Change | Required? |
|---|---|---|---|
| `manifest.rs:1384-1405` | `protocol_auth_schemes_match_strict_agents_and_flexible_http_transport` | add `("openai.responses", vec!["bearer"])` **and** a new structural assertion: every descriptor with `executor == Agent` has exactly one allowed auth scheme | scheme row optional; **the structural assertion is the point** — it closes the silent class for the next Agent protocol |
| `manifest.rs:1176-1184` | `default_manifest_matches_both_executor_registries` | add `registry.get("openai.responses").is_some()` | no (`.is_some()` list) |
| `tests/protocol_manifest.rs:69-104` | `public_registry_is_enumerable_consistent_and_duplicate_safe` | add the auth-scheme assertion | no |
| `config.rs:2276-2282` | `test_provider_label_is_builtin_name_for_builtin` | add the alias row | no, but the arm ships untested without it |
| `provider_config.rs:166-168` | enumerated error text | list five protocols | **yes** (product text) |
| `tests/provider_config_protocol_contract.rs:176-245` | `…resolves_nomi_config` | add an `OpenAIResponses` case | **yes** (policy) |
| `nomifun-system/tests/agent_chat_protocol_contract.rs:1`, `:88-170` | module doc says "the four Agent Chat protocol families"; `valid`/`invalid` are fixed arrays | doc → five; add valid `http_provider("openai","bearer","openai.responses","gpt-contract","/responses")` and invalid `header_key:x-api-key` variant | **yes** (policy). Honest note: **this does not break compile or pass** — the "exactly four" is prose, not an assertion. Calling it a required break, as the prior draft did, overstates the blast radius. |

**Explicitly unchanged (verify, do not edit)**

- `manifest.rs:1461-1473`, `:1478`; `tests/protocol_manifest.rs:143-156`, `:184` — protocol lists/counts, **unchanged** because of `NATIVE_ONLY`. If any of these move, `scopes` was set wrong.
- `manifest.rs:1566-1613` and `tests/protocol_manifest.rs:255-315` — locked URL-snapshot hash `9_446_312_405_170_401_367`. Both iterate `view.recommendation` only, and `routes_table.rs:26` keeps `Chat => route("openai.chat_text")`. **If either hash moves, `routes_table.rs` was changed by mistake.**
- `manifest.rs:1428`, `:1433`; `tests/protocol_manifest.rs:133`, `:138`; `nomifun-system/src/routes.rs:221` — all `RealtimeConversation`, genuinely unaffected.
- `crates/backend/nomifun-db/migrations/**` — **no migration.** `032_provider_model_capabilities.sql:60` has no value allowlist; the `known_protocol_tasks` VALUES list at `:100-108` is a one-time backfill CTE. `UNIQUE (provider_id, model, task)` at `:76` makes `openai.chat_text` and `openai.responses` mutually exclusive per model — you cannot half-migrate a model.
- `nomi-providers/src/lib.rs:94-123` `ProviderError::redacted` — exhaustive, but no variant is added.
- `ui/src/renderer/utils/model/modelPlatforms.ts` — no new preset, so `backend_catalog_covers_every_ui_model_platform_preset` (`manifest.rs:1512-1542`, plus its copy) stays green.
- `ModelDefinitionEditor.tsx:610`/`:783-797` and `providerModelAdvanced.ts:296-300`/`:562-565` — the protocol dropdown and its validation are manifest-driven; **no protocol enumeration exists in `ui/src` production code** (`openai.chat_text` appears only in test fixtures).
- `nomi-agent/tests/badcase_regression_test.rs:126-135` `scripted_server` hardcodes `path("/v1/chat/completions")` and is shared by `:337/:390/:406/:464/:475`. **Do not parameterize it.** The new integration test (§12) brings its own server.

---

## 10. What old code is DELETED

Small on purpose — this workstream is additive except where the corrected design makes prior mechanism unreachable.

1. `crates/agent/nomi-providers/src/openai.rs` — the eight moved items at `:329`, `:338-363` (extracted), `:480`, `:496`, `:1083`, `:1102`, `:1123`, `:1391` are **removed from this file** and re-exported from `openai_shared.rs`. `openai.rs` keeps no copy; call sites are re-pointed. Pure move.
2. **Nothing else is deleted from the tree by this workstream.** The global output-ceiling deletions (`NomiBuildExtra.max_tokens` → its single reader `factory/nomi.rs:671`; `ui/src/common/config/storage.ts:121 maxTokens`) belong to C1.

Deleted *relative to the prior draft*, so the reviewer does not implement dead mechanism: `LlmRequest.provider_round`; `struct ProviderRoundChain`; `AgentEngine.chained_prefix_len`; `AgentEngine.chain_provider_rounds`; `AgentEngine.chain_recovered_this_turn`; `HOST_CONTEXT_RESPONSES_PARENT` and `…_PREFIX_LEN`; the separate "commit after terminal validation" step; the seven-row chain-break table keyed on `host_context`; the `set_host_context_values` batch setter critique_3 asked for; the "last-wins across a retry" rule; the `routes_table.rs` negative test (redundant with the locked hash); the `scopes: NATIVE_CUSTOM` decision and its six assertion edits.

---

## 11. Coordination with the other two workstreams

**`LlmRequest` fields I own: none.**
I do not add, remove, retype, or move any field of `LlmRequest` (`llm.rs:8-18`), and I touch none of the 17 literals. The output-ceiling workstream owns `max_tokens: u32 → Option<u32>` and all 17 sites, alone, with zero merge interaction. My only dependency is a **consumer** contract, pre-commented at one line inside `openai_responses.rs::build_request_body` (§7.3): when `max_tokens` becomes `None`, omission must also `object.remove(max_tokens_field)` **after** `request_body_with_extra`, because `lib.rs:46-50` merges `compat.extra_body()` first and the typed body only wins for keys it actually inserts (proven by `provider_openai_test.rs:954→970`, `provider_anthropic_test.rs:606→624`, `bedrock.rs:764→801`). The removal I already write for `tools`/`reasoning`/`previous_response_id` gives that edit its exact shape.

I also do not add a `reasoning_tokens` field to `TokenUsage` (`message.rs:116-125`); I capture and log it. C1 owns that type.

**`Session.host_context` keys I own: none.**
The resumable-round workstream keeps sole ownership of `host_context` (`session.rs:59-64`, engine accessors `:827`/`:833`). This is the deliberate consequence of moving the parent pointer onto `Message`. There is no key to coordinate, no "one key per concern" contract to enforce, and no requirement that B1 clear anything of mine on rewind or edit — `EditableTurnCheckpoint.prior_host_context` (`:1225`, `:1314`, restored `:2646`) needs no new member.

**What I do own, exclusively:** `Message.provider_round_id` + `nomi_types::message::clear_provider_round_ids`; `LlmEvent::ProviderRoundId`; `ProviderCompat.chain_rounds` + accessor + `openai_responses_defaults()`; `ProviderType::OpenAIResponses`; `NomiCompatOverrides.chain_rounds`; `openai_responses.rs` and `openai_shared.rs`; `ProviderError::is_stale_previous_response`; the `openai.responses` `ProtocolSpec` and its four manifest gates; `CROSS_PROTOCOL_ENDPOINT_PATHS`/`conflicting_endpoint_owner`.

**Shared-file merge points, with the resolution:**
- `engine/mod.rs` — B1 also edits the turn loop. My edits are: one `let` after `:1468`, one match arm after `:1711`, the `:1898-1907` rewrite, and five one-line calls in the rewrite helpers (`:2324`, `:2359`, `:2381`, `:2422`, and the two `prune_old_tool_images` call sites). None of these are inside B1's `MaxTokens` restart logic. If B1 introduces its own restart at the `:1745-1763` validation band, my design is unaffected: I do not commit anything there.
- `manager/nomi/agent.rs:970-972` and `provider_config.rs:266-268` — I add one `if let Some(chain) = …` line to each existing per-field copy block. C1 does not touch these blocks.

**Why not a field on `LlmEvent::Done` — the cheapest correct capture point.**
`LlmEvent::Done { … }` has **197 construction sites** (`grep -rn "LlmEvent::Done {" crates/ | wc -l` = 197, exactly matching the total `LlmEvent::Done` reference count, i.e. every reference is a literal). A new field breaks all 197. A new variant breaks **exactly two** exhaustive matches (`engine/mod.rs:1534`, `image_generation.rs:185`), needs zero changes to `SecretRedactingProvider` (`lib.rs:427` `other => other`), and is auto-excluded from the TTFT gate and the replay-unsafe set (both `matches!`). Ratio: 2 vs 197.

But the arithmetic is not the decisive argument, and neither is the prior draft's ("the id arrives first, at `response.created`, so coupling it to `Done` throws that away"). The decisive argument is **semantic**: with a `Done` field, "the provider produced an id" and "the id is a legal chain parent" become the same statement, and §1.1 condition 3 has nowhere to live — you would be forced to re-derive "did the round leave open tool calls?" in the engine, which cannot know. A separate variant lets the provider *withhold* the id. Withholding is the entire state machine.

---

## 12. Test plan

**`cargo test -p nomifun-model-invoke`** (`bun run test:crate nomifun-model-invoke`)
- boot: `try_default_protocol_registry()` is `Ok` and `registry.get("openai.responses")` is `Some` with `root_shape == VersionedRoot`, `scopes == [Native]`, `platforms == ["openai"]`, `allowed_auth_schemes == ["bearer"]`
- **new structural**: every descriptor with `executor == Agent` has exactly one allowed auth scheme
- `every_manifest_protocol_task_has_a_provider_params_encoding_contract` still passes (`:1186`)
- `protocol_manifest_for("OpenAI", Chat)` contains `openai.responses`; `protocol_manifest_for("StepFun", Chat)`, `("custom", Chat)` and `("gemini", Chat)` do **not**
- `chain_rounds`: `true`/`false` accepted on `("openai.responses", Chat)`; `1`/`"true"` rejected; **any** value rejected on `openai.chat_text`, `anthropic.messages`, `stepfun.images`
- footgun: `/responses` on `openai.chat_text` rejected with a message naming `openai.responses`; `/chat/completions` on `openai.responses` rejected symmetrically; each accepted on its own owner; `/custom/chat?api-version=2026-08-11` accepted on both; `mimo.chat_asr` + `/chat/completions` still validates
- both locked URL-snapshot hashes unchanged (`manifest.rs:1609-1612`, `tests/protocol_manifest.rs`)

**`cargo test -p nomi-types`** — `Message` with `provider_round_id: Some("resp_x")` round-trips; `None` omits the key entirely (mirrors `test_message_new_skips_timestamp_in_json`, `message.rs:476-489`); a session JSON without the field deserializes to `None`; `clear_provider_round_ids` returns `true` iff it removed at least one and leaves `content` byte-identical.

**`cargo test -p nomi-config`** — `openai_responses_defaults()` field-by-field; `ProviderCompat::merge` carries `chain_rounds` both directions; `parse_builtin_provider("openai-responses")`; `compat_defaults()` for the new `ProviderType`.

**`cargo test -p nomi-providers --test provider_openai_responses_test`** (new file)
- body: `input` / `instructions` (not a `role:"system"` item) / **flat** tools / `max_output_tokens` / `reasoning.effort` only when set
- **bootstrap**: round 1 with `chain_rounds:true` asserts `store == true` **and** `previous_response_id` absent **and** `input` is a full snapshot
- **`chain_rounds:false`**: `store == false`, `previous_response_id` absent, on **every** round
- **Mode C delta**: with a parent id on the newest assistant message, `previous_response_id` is sent and `input` contains **neither** the earlier user message **nor** the parent's assistant text — the anti-double-count assertion
- **Mode C filter bypass**: a delta consisting solely of `function_call_output` items survives `build_input` unmodified even though `clean_orphan_tool_calls`/`dedup_tool_results` are `Some(true)`
- **not armed**: a parent id on an *older* assistant while the newest has none → full snapshot, no `previous_response_id`
- SSE → `LlmEvent` for every row of §7.4
- terminal: `status:"incomplete"` + `max_output_tokens`, zero tool calls → `Done{MaxTokens}` **and** `ProviderRoundId` emitted immediately before it
- terminal: `status:"incomplete"` + `max_output_tokens` with one `function_call` item → `Done{MaxTokens}`, accumulators dropped, warn logged, **no `ProviderRoundId`** (the §1.1(3) regression guard)
- `content_filter` → `Error`; missing/unknown `status` → `Error`; unknown `response.*` before terminal ignored; **any** event after terminal poisons
- recovery: first POST returns 400 `"Previous response with id 'resp_x' not found"` → **exactly two** requests recorded; the second has no `previous_response_id`, a full-snapshot `input`, and still `store:true`; a `ProviderRoundId` is emitted from it
- recovery is one-shot: two consecutive stale-parent 400s surface the second as `ProviderError::Api`
- 404 on submit → non-retryable `Api{404}` whose message names `openai.chat_text`
- `usage.output_tokens_details.reasoning_tokens` does not appear in `TokenUsage`

**`cargo test -p nomi-providers`** — the `openai_shared` move is behaviour-neutral: the existing `provider_openai_test.rs` and `openai.rs` unit tests pass untouched. Plus: `SecretRedactingProvider` forwards `ProviderRoundId` byte-identically.

**`cargo test -p nomi-agent --test responses_chaining_test`** (new file; its own `MockServer` mounting `POST /v1/responses`, its own `RecordingResponder`, `ProviderType::OpenAIResponses` in the `Config` literal — **does not touch `badcase_regression_test.rs`'s shared `scripted_server`**)
- two-round tool turn: round 2's recorded body carries `previous_response_id` and an `input` of exactly the `function_call_output` item
- a round whose `supersede_written_draft` fired: the pushed assistant message has `provider_round_id == None`, and round 2's body has no `previous_response_id` and a snapshot `input` containing the **collapsed** text
- a round that errors after `Done` (forced `done_count != 1`): after `:1188` rollback, no message in `self.messages` carries a round id
- microcompact fires between rounds → next body is a snapshot
- autocompact fires between rounds → next body is a snapshot; `editable_turn` is `None` as before
- `rewind_last_turn` then a new turn → snapshot
- save → `resume_with_provider` after `sanitize_session_messages` → snapshot (the stated resume limit, pinned)

**`cargo test -p nomifun-ai-agent`** — `sanitize_session_messages` clears every round id including in the split-half path; `provider_config` resolves `openai.responses` → `provider == "openai-responses"`, `api_path == Some("")`, `chain_rounds == Some(true)`, and `extra_body` does **not** contain `chain_rounds`; `chain_rounds` as a non-boolean is a 400; **`build_probe_engine` leaves `config.compat.chain_rounds == None` even when the capability sets it** (`provider_health.rs`).

**`cargo test -p nomifun-ai-agent --test provider_config_protocol_contract`** — the fifth resolution case.

**`cargo test -p nomifun-system --test agent_chat_protocol_contract`** and **`cargo test -p nomifun-system provider_model`** — save-time: `openai.responses` + `bearer` on the `openai` platform → 201; `openai.responses` + `header_key:x-api-key` → 400; **`openai.responses` on the `stepfun` platform → 400 naming the platform** (`provider_model.rs:263-273`); `/responses` typed on `openai.chat_text` → 400.

**`bun run test:ui`** — extend `providerModelAdvanced.test.ts` (`providerParamChainRounds`/`withProviderParamChainRounds` round-trip, unparseable JSON is left byte-identical, unsetting removes the key and empties the JSON when it was the only key) and `ModelDefinitionEditor.render.test.tsx` (the control renders for `openai.responses` and not for `openai.chat_text`; the retention copy is present; disabled when the JSON is invalid). **Known pre-existing failure:** `bun run test:ui` has one unrelated `CreateStudio` modal failure from upstream restyling — not caused by this work.

Not run in this scope: the full `cargo test` sweep. The `nomifun-app` lib suite has a known flake — exactly one rotating test fails per run at `.send()` and passes in isolation — so the per-crate commands above are the meaningful gate for this change.

---

## 13. How this design behaves on the observed failure

The reported failure is unreachable from `openai.responses` (`step-3.7-flash` on StepFun; `manifest.rs:285-286`, no `/responses`, and after §5 the protocol is rejected at save time for that platform with an exact message rather than 404-ing at first invocation). Traced with the provider swapped for OpenAI and `chain_rounds:true`, holding A1/B1/C1 constant so this workstream's contribution is isolated:

1. **Turn 1, pass 1.** `engine/mod.rs:1453` builds the request. The transcript's newest assistant message does not exist yet → `chain_parent` returns `None` → **snapshot** `input`, `store:true`, **no** `previous_response_id`. (Under the prior draft this pass sent `store:false` and every subsequent pass was also pass 1.)
2. The reasoning model burns the whole `max_output_tokens` budget. The terminal event carries `status:"incomplete"`, `incomplete_details.reason:"max_output_tokens"`, `output` with **zero** `function_call` items. `terminal_stop` → `StopReason::MaxTokens`. Note what did *not* happen: there is no `finish_reason` string to fold, so the `normalize_finish_reason` `content_filter → "stop" → EndTurn` path (`openai.rs:1399-1417`) that turns a stop into `result_ok=1` has no analogue here.
3. §1.1 is satisfied on all three conditions (`store:true`; `incomplete/max_output_tokens`; no dropped accumulators). `drain_terminal_events` emits `ProviderRoundId("resp_A")` then `Done{MaxTokens}`.
4. Engine: `:1745` `done_count == 1` ✓; `:1750-1761` `MaxTokens` with empty `tool_calls` ✓; `:1898` `supersede_written_draft` returns `false` (no write in this round); `:1906-1907` pushes the assistant message **carrying `provider_round_id: Some("resp_A")`**, and `save_session()` persists it in the same write as the message.
5. `execute_turn_inner` returns `AgentResult{stop_reason: MaxTokens}`. **A1 makes this not a success** and B1 owns the restart; `manager/nomi/agent.rs:2058-2077` re-invokes the turn with `truncation_continuation_prompt(1, 2, …)` as a new `Role::User` message. `editable_turn` is preserved (`:1307-1316` only re-creates it when `source_message_id` changes).
6. **Pass 2.** `chain_parent` finds `resp_A` on the newest assistant message; `delta_from` is the index after it; `delta_from < messages.len()` because the continuation prompt was appended. Request: `previous_response_id: "resp_A"`, `store: true`, `input: [ the continuation prompt only ]`.
   * **Input tokens for this pass collapse from the whole transcript to one message.** In the observed session that is the difference between re-billing ~25k input tokens per recovery pass and billing ~200.
   * The model's reasoning items from pass 1 are retained server-side, so it **resumes** the interrupted plan instead of re-deriving it from scratch — which Mode S structurally cannot do, since Mode S drops reasoning (`include: ["reasoning.encrypted_content"]` round-tripping is out of scope, §0).
   * There is no double-read: the truncated prose is in server state and **not** re-sent in `input`. Under a naive `previous_response_id` + full `messages` combination it would be counted twice and the model would read its own runaway output twice — the mechanism named in the critique, and the reason `input` is a delta and never a snapshot in Mode C.
7. **If pass 2 truncates mid-tool-call instead** — the shape `agent.rs:720-733`'s own prose anticipates ("If a file write or tool argument was interrupted") — the terminal `output` holds a `function_call` item, the engine's `:1750-1761` validator forces the accumulator to be dropped, §1.1 condition 3 fails, **no `ProviderRoundId` is emitted**, the assistant message carries no id, and pass 3 is a full snapshot. The parent with unanswered calls is never chained to, so the guaranteed 400 the critique identified cannot occur. Cost: pass 3 pays full input tokens. That is the honest price of the limit in §0.
8. **If pass 2's round writes the deliverable and `supersede_written_draft` collapses its pre-tool draft**, §3(i) withholds the id, and pass 3 sends a full snapshot **of the collapsed transcript** — so the collapse actually reaches the wire. Chaining does not defeat it, and no history rewrite is silently disabled anywhere in this design.
9. **If the stored parent has expired** (a resumed session on the CLI path, or a 30-day-old chain), the first POST 400s; §1.3(a) drops `chain` and re-POSTs once with a full snapshot, still `store:true`, and that round's id shadows the stale one on the newest assistant message. One extra request, one `warn!`, no engine involvement, no recurring waste.
10. **What this workstream does not do to the receipt.** `relay_error_code.rs:73-79` `turn_succeeded(Finish, non-blank prose, 0) == true` is A1's to fix. `openai.responses` makes truncation *unambiguous at the provider boundary* (a two-level object lookup with no string bucket to fold) and *cheaper to recover from*, and captures `reasoning_tokens` into a log line so the receipt investigation has the 24576-token accounting available. It does not change what the relay records, and §0 says so. This workstream must not be cited as fixing the reported failure.