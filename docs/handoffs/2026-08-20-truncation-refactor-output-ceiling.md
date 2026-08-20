I have everything verified. Writing the corrected design.

# OUTPUT-TOKEN-CEILING — CORRECTED DESIGN (C1)

Verified against `HEAD = c603bd7f`, repo root `C:/Users/rika0/code/nomifun/bak/1/nomifun-tauri`. Every line number below was opened.

**Note that changes the plan:** A1 has already landed. `crates/backend/nomifun-conversation/src/relay_error_code.rs:26` already defines `OUTPUT_TRUNCATED = "output_truncated"`, `:97-105 incomplete_stop_code` already maps `TurnStopReason::MaxTokens → OUTPUT_TRUNCATED`, `:121-129 turn_succeeded` already excludes it, and `:41-51 fixed_code_retryable` already returns `true` for it. So the first round's proposed `output_token_ceiling` relay code is a **duplicate** and is deleted from this design.

---

## 1. Blocker 3 — the `extra_body` escape hatch

### Decision: BOTH. Strip at the compat boundary (the wire guarantee) **and** reject at save-time validation (the diagnostic). They do different jobs and neither substitutes for the other.

Why strip is mandatory and must be unconditional:
`crates/agent/nomi-providers/src/lib.rs:47-50` builds the body from `compat.extra_body()` first and then overlays the typed body. The typed `max_tokens` wins today **only because `openai.rs:279` inserts it unconditionally**. Three existing tests prove `extra_body` really does carry `max_tokens` in the field: `crates/agent/nomi-providers/tests/provider_openai_test.rs:953` puts `"max_tokens": 1` in `extra_body` and `:970` asserts the typed `512` wins; same shape at `tests/provider_anthropic_test.rs:607/:624` and `src/bedrock.rs:764/:801`. The moment the typed insert becomes conditional, `provider_params.max_tokens` is promoted from *always discarded* to *the effective ceiling on exactly the omission path* — the worst possible outcome, and the precise inverse of the argument that justified putting the ceiling in a typed column. Nothing rejects it today: `crates/backend/nomifun-model-invoke/src/manifest.rs:750-770` special-cases only `max_tokens_field` and `require_reasoning_content`, and `crates/backend/nomifun-ai-agent/src/factory/provider_config.rs:177-196` removes only those same two keys before `:208` dumps the remainder into `extra_body`.

Why reject is also mandatory: `validate_provider_params_for_protocol`'s own contract (`manifest.rs:717-720`) is *"a parameter can never save successfully and then be silently discarded by an executor"*, and `manifest.rs:742-748` already rejects `stepfun.images` `generation_option_keys` for exactly that reason. Strip-alone re-creates the bug that validator exists to prevent. Reject-alone leaves every already-saved row, every gateway-authored row (`crates/backend/nomifun-gateway/src/caps_system.rs`), and every hand-edited DB able to smuggle a ceiling.

### 1a. The strip — pre-merge, in the single funnel

`request_body_with_extra` is `pub(crate)` and is the only merge point, called from exactly four adapters: `anthropic.rs:103`, `bedrock.rs:112`, `gemini.rs:174`, `openai.rs:292`. `vertex.rs` never calls it and therefore has no `extra_body` exposure at all.

```rust
// crates/agent/nomi-providers/src/lib.rs — replaces :43-50

/// Where a protocol's request body carries the output ceiling, so the merge can
/// guarantee `extra_body` never supplies it. The typed field is the ONLY
/// authority: when it is absent the field must be ABSENT, not silently
/// inherited from a saved `provider_params` value.
///
/// Passed explicitly rather than matched from a key-name list so a future
/// adapter with a new field name cannot forget to close the hole.
#[derive(Debug, Clone, Copy)]
pub(crate) enum OutputCeilingKey<'a> {
    /// Top-level, e.g. `max_tokens` / `max_completion_tokens`.
    Top(&'a str),
    /// Nested one object deep, e.g. `generationConfig.maxOutputTokens`.
    /// The parent object is removed if stripping empties it.
    Nested { parent: &'a str, key: &'a str },
}

/// Merge provider-native body extensions first, then recursively overlay the
/// serializer's typed protocol body. The output ceiling is removed from the
/// extension map BEFORE the merge — post-merge removal would also delete a
/// legitimately present typed value.
pub(crate) fn request_body_with_extra(
    compat: &ProviderCompat,
    ceiling_key: OutputCeilingKey<'_>,
    typed: Value,
) -> Value {
    let mut extra = compat.extra_body();
    match ceiling_key {
        OutputCeilingKey::Top(name) => {
            extra.remove(name);
        }
        OutputCeilingKey::Nested { parent, key } => {
            if let Some(Value::Object(nested)) = extra.get_mut(parent) {
                nested.remove(key);
                if nested.is_empty() {
                    extra.remove(parent);
                }
            }
        }
    }
    let mut body = Value::Object(extra);
    merge_json_value(&mut body, &typed);
    body
}
```

Nested handling is required because `merge_json_value` (`lib.rs:28-41`) is recursive: a saved `{"generationConfig":{"maxOutputTokens":99}}` would deep-merge with Gemini's typed `generationConfig` (`gemini.rs:155-159`) and survive on the omission path.

Call-site changes (4):
- `anthropic.rs:103` → `crate::request_body_with_extra(&self.compat, OutputCeilingKey::Top("max_tokens"), body)`
- `bedrock.rs:112` → same
- `gemini.rs:174` → `OutputCeilingKey::Nested { parent: "generationConfig", key: "maxOutputTokens" }`
- `openai.rs:292` → `OutputCeilingKey::Top(max_tokens_field)` using the already-resolved local from `openai.rs:260-264` (**not** the literal `"max_tokens"` — the resolved name can be `max_completion_tokens`, set at `provider_config.rs:177-186`)

Consequence: `"max_tokens": null` is structurally impossible. The key is either present with a `u32` or absent. `openai.rs:279` becomes a conditional insert (`if let Some(limit) = request.max_tokens { body[max_tokens_field] = json!(limit); }`), and there is no `object.remove` for it in the post-merge block at `openai.rs:293-301`.

### 1b. The rejection — save time

```rust
// crates/backend/nomifun-model-invoke/src/manifest.rs, inside
// validate_provider_params_for_protocol's `if task == Chat` block (:750)

/// Every request-body name any Chat adapter uses for the output ceiling, plus
/// the caller's own resolved `max_tokens_field`. The ceiling is a typed column
/// (`provider_model_capabilities.output_limit`); a value here is stripped at the
/// compat boundary, so accepting it would be accepting a no-op field.
const OUTPUT_CEILING_PARAM_KEYS: &[&str] = &[
    "max_tokens", "max_completion_tokens", "maxOutputTokens", "max_output_tokens",
];
```
Reject any of those keys, plus the string value of `max_tokens_field` if present, with:
`"Chat provider_params must not set {key:?}; the output ceiling is the capability's \"Max output tokens\" field (output_limit)"`.

This is also the reason the UI's `providerParamsJson` free-text box cannot be used as a back door.

---

## 2. Blocker 4 — where the per-protocol requirement lives

### 2a. Why NOT `resolve_provider_fields`

`crates/backend/nomifun-ai-agent/src/factory/provider_config.rs:68` has exactly three non-test callers:
- `factory/nomi.rs:465` — the chat agent build (the only path that can emit `None`)
- `provider_config.rs:234` (`resolve_provider_config`) → **8 production callers**: `nomifun-agent-execution/src/planner.rs:228`, `nomifun-ai-agent/src/knowledge_completer.rs:70`, `one_shot.rs:75`, `terminal_title_completer.rs:64`, `nomifun-app/src/robot_wiring.rs:50`, `nomifun-app/src/services.rs:64`, `nomifun-companion/src/learner.rs:92`, `nomifun-idmm/src/sidecar.rs:43`
- `services/provider_health.rs:148` — the health probe

Every one of those supplies its own explicit per-request ceiling (`one_shot.rs:131 ONE_SHOT_MAX_TOKENS`, `image_generation.rs:172 max_tokens: 96`, `provider_config.rs:301/:313/:346 max_tokens: u32`, `provider_health.rs:159 max_tokens: 16`) and therefore *structurally cannot* emit `None`. A gate keyed on the **declared** limit there rejects all nine paths for any Anthropic/Bedrock row with `output_limit` NULL — including the probe, so the user cannot even diagnose it. It would also fail four existing tests: `crates/backend/nomifun-ai-agent/tests/provider_config_protocol_contract.rs:179,199,216,233` resolve all four Chat protocols, and the capability fixture at `:67-75` sets no `output_limit`.

`resolve_provider_fields` therefore stays **total**. It gains one field and nothing else.

### 2b. The rule itself lives on `ProviderType`

`nomi_config::config::ProviderType` (`crates/agent/nomi-config/src/config.rs:502-508`) is the 5-variant enum that `create_provider` (`crates/agent/nomi-providers/src/lib.rs:603-656`) matches 1:1 to select the adapter. Putting the rule there makes it structurally impossible to drift from the serializer, and the compiler forces a decision if a 6th adapter is added.

```rust
// crates/agent/nomi-config/src/config.rs — new method in `impl ProviderType` (:510)

/// Whether this protocol's request body MANDATES the output ceiling.
///
/// The Anthropic Messages body (`anthropic.rs:74`, `bedrock.rs:83`,
/// `vertex.rs:79`) is rejected without `max_tokens`. OpenAI-compatible
/// (`openai.rs:279`) and Gemini (`gemini.rs:159`) apply their own maximum when
/// the field is omitted, which is the whole point of making absence
/// representable. Exhaustive on purpose: a new adapter must decide.
pub fn requires_output_ceiling(self) -> bool {
    match self {
        ProviderType::Anthropic | ProviderType::Bedrock | ProviderType::Vertex => true,
        ProviderType::OpenAI | ProviderType::Gemini => false,
    }
}
```

This **deletes** the first round's `OutputCeilingPolicy` enum, the `LlmProvider` trait method, the `SecretRedactingProvider` forwarding, and the test that pinned the forwarding. Both critiques were right that they had no reader (layers 1/2/3 all read something else), and `SecretRedactingProvider` (`lib.rs:405-435`, one forwarded method) could never have disabled the adapter's own guard anyway.

### 2c. The pre-turn gate — exact function and signature

The gate goes where the *effective* ceiling is known, which is the one function that finalizes it for a long-lived chat session. Today that is `apply_provider_context_budget` (`crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs:55-62`), called at `:984` and nowhere else in production (verified: `grep fit_context_budget|apply_provider_context_budget` returns only `compact.rs:145`, `agent.rs:55/61/984/4459`).

```rust
// crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs:55 — replaces
// apply_provider_context_budget wholesale

/// Fit the provider's declared window and output ceiling onto the resolved
/// `Config`, and enforce the protocol's own requirement before the turn starts.
///
/// `config.output_max_tokens` arrives already composed by `Config::resolve`
/// (`nomi-config/src/config.rs:606`: `cli.max_tokens.or(default.max_tokens)`),
/// where the host's CliArgs leg carries the capability's declared ceiling. This
/// is the SINGLE writer of the field; there is no second combinator.
///
/// `Ok(())` with `config.output_max_tokens == None` means the serializer omits
/// the field and the provider applies its own maximum. For protocols whose body
/// mandates it we fail HERE — before the first token is spent — with a message
/// that names the exact control to fix.
fn apply_provider_token_budget(
    config: &mut Config,
    context_limit: Option<u64>,
) -> Result<(), AppError> {
    config.compact.context_window = nomi_config::compact::resolve_context_window(
        context_limit,
        config.compact.context_window,
    );
    config.output_max_tokens =
        nomi_config::compact::fit_context_budget(&mut config.compact, config.output_max_tokens);

    if config.output_max_tokens.is_none() && config.provider.requires_output_ceiling() {
        return Err(AppError::BadRequest(format!(
            "the {} protocol requires an explicit output ceiling; set \
             “Max output tokens” on the {} chat capability in Settings → Models",
            config.provider_label, config.model
        )));
    }
    Ok(())
}
```

Call site `agent.rs:984` becomes `apply_provider_token_budget(&mut config, config_extra.context_limit)?;` — the enclosing function already returns `Result<_, AppError>` (`agent.rs:955` uses `map_err(...)?`), so `?` is legal with no signature churn.

`provider_health.rs` **never calls this function**, which is exactly why it is the right home. The probe's own ceiling reaches the wire unchanged through its own `CliArgs` at `provider_health.rs:397`.

### 2d. The two `NomiResolvedConfig.max_tokens` production readers the first round missed

Both are `CliArgs` literals, and they are the *only* paths by which `NomiResolvedConfig.max_tokens` (`types.rs:124`) reaches a `Config`:
- `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs:948` — `max_tokens: Some(config_extra.max_tokens)`
- `crates/backend/nomifun-ai-agent/src/services/provider_health.rs:397` — `max_tokens: Some(config_extra.max_tokens)`

`NomiResolvedConfig.max_tokens: u32` is **retyped**, not split. There is no `request_output_ceiling` second channel (it would have had exactly one producer and zero readers):

```rust
// crates/backend/nomifun-ai-agent/src/types.rs:123-124
/// Output ceiling for this runtime's requests. `None` = undeclared; the
/// serializer omits the field and the provider applies its own maximum.
///
/// Chat sessions carry the model's DECLARED ceiling from
/// `provider_model_capabilities.output_limit`. The health probe carries its own
/// 16-token per-request budget. Both are the same role at this altitude — a
/// number this runtime is willing to have on the wire — and both flow through
/// `CliArgs.max_tokens`, which is already `Option<u32>` (`config.rs:555`).
pub output_ceiling: Option<u32>,
```

- `factory/nomi.rs:671` → `output_ceiling: fields.output_limit.map(|v| v as u32)` (deletes `max_tokens: overrides.max_tokens`)
- `provider_health.rs:159` → `output_ceiling: Some(16)`
- `agent.rs:948` → `max_tokens: config_extra.output_ceiling` (drops the `Some()`)
- `provider_health.rs:397` → `max_tokens: config_extra.output_ceiling` (drops the `Some()`)

The TOML/CLI leg now genuinely composes: `Config::resolve` at `config.rs:606` becomes `cli.max_tokens.or(merged.default.max_tokens)`, so a CLI `--max-tokens` still wins over `[default] max_tokens`, and on the desktop path the capability's declared ceiling arrives as `cli.max_tokens`. One writer, one precedence chain.

### 2e. The authoring flag — one derived function, no `ProtocolSpec` churn

`crates/backend/nomifun-model-invoke/src/manifest.rs` has no `nomi-config` or `nomi-providers` dependency (verified against its `Cargo.toml`), so it cannot call `ProviderType::requires_output_ceiling`. It gets its own function, derived from `spec.id` inside `owned_protocol` exactly the way `allowed_auth_schemes` already is (`manifest.rs:424-444`, consumed at `:900`):

```rust
// crates/backend/nomifun-model-invoke/src/manifest.rs, beside allowed_auth_schemes (:424)

/// Whether a Chat protocol's request body mandates the output ceiling, so the
/// model editor can require it at authoring time. Mirrors
/// `nomi_config::config::ProviderType::requires_output_ceiling`; the two are
/// pinned together by
/// `nomifun-ai-agent/tests/provider_config_protocol_contract.rs`, which is the
/// only crate that can see both.
pub fn protocol_requires_output_ceiling(protocol_id: &str) -> bool {
    matches!(protocol_id, "anthropic.messages" | "bedrock.anthropic_messages")
}
```

Exported alongside `protocol_descriptor` at `crates/backend/nomifun-model-invoke/src/lib.rs:47`, and used for `ProtocolDescriptor.requires_output_ceiling: bool` populated at `manifest.rs:900`. **All 35 `ProtocolSpec` static rows are untouched.**

`vertex.*` is not a registered protocol — `grep -c vertex crates/backend/nomifun-model-invoke/src/manifest.rs` = **0**, and `provider_config.rs:165-169` rejects anything outside the four Chat protocols. `vertex.rs` is reachable only from `nomi-cli`, so it gets the `ProviderType` gate (via `Config::resolve` → `apply_…`? no — the CLI does not call `apply_provider_token_budget`) and the adapter guard only. Stated plainly rather than implied.

---

## 3. The compactor decoupling

### 3.1 What was actually wrong

`crates/agent/nomi-config/src/compact.rs:152` is `config.output_reserve = config.output_reserve.max(max_tokens as usize)` and `:159` is `config.output_reserve = max_tokens as usize`. The second one is the real blocker: the valve's **collapse target** is the ceiling, so `None` has nothing to collapse to. The first one is harmless (a `max` against 0).

### 3.2 The corrected derivation

```rust
// crates/agent/nomi-config/src/compact.rs — moved down from
// nomi-agent/src/compact/prompt.rs:11-21 (nomi-config cannot depend on
// nomi-agent: nomi-agent/Cargo.toml:21 depends on nomi-config)

/// One response's worth of context window, in tokens.
///
/// The single unit for "how much window does one answer occupy". Used as the
/// compactor's own summary budget (`compact/auto.rs:126`) and as the structural
/// input-headroom floor / valve collapse target here, so the two can never
/// disagree. Window-only by construction, which is what keeps it defined when
/// the model's output ceiling is undeclared.
///
/// 200k -> 20_000 (== the historical `default_output_reserve()`), 128k -> 16_000,
/// 32k -> 4_000, 1M -> 20_000 (clamped), 4096 -> 512.
pub fn window_output_unit(context_window: usize) -> u32 {
    u32::try_from((context_window / 8).max(1))
        .unwrap_or(u32::MAX)
        .min(20_000)
}

/// Fit the request ceiling and the compaction budgets inside the window.
///
/// `declared_output_limit` is what this runtime is willing to put on the wire
/// (the capability's `output_limit`, or a caller's `--max-tokens`). `None` =
/// undeclared; the returned ceiling is `None` and the serializer omits the field.
///
/// `output_reserve` is INPUT HEADROOM. It is the caller's configured value
/// (`compact.output_reserve`, a documented TOML key), floored by
/// `window_output_unit` so a response always has somewhere to go, and raised to
/// whatever we actually permit the provider to emit. It is never DEFINED by the
/// ceiling — that inversion is what made `None` unrepresentable.
pub fn fit_context_budget(
    config: &mut CompactConfig,
    declared_output_limit: Option<u32>,
) -> Option<u32> {
    let context_window = config.context_window.max(1);

    // A single response may never claim more than a quarter of the window.
    let ceiling_cap = u32::try_from((context_window / 4).max(1)).unwrap_or(u32::MAX);
    let request_ceiling = declared_output_limit.map(|limit| limit.min(ceiling_cap));
    let permitted = request_ceiling.map_or(0usize, |limit| limit as usize);

    // Window-only. Always >= 1, so the reserve can never be zero.
    let structural = window_output_unit(context_window) as usize;

    config.output_reserve = config.output_reserve.max(structural).max(permitted);

    // Sanity valve: 200k-tuned defaults must not starve a small provider. The
    // collapse target is the STRUCTURAL unit, raised to the permitted ceiling —
    // window-derived, so it is defined when the ceiling is None. This is the one
    // line that made the old signature impossible.
    if config.output_reserve.saturating_add(config.autocompact_buffer) > context_window / 2 {
        config.output_reserve = structural.max(permitted);
        config.autocompact_buffer = config.autocompact_buffer.min((context_window / 8).max(1));
    }

    config.emergency_buffer = config.emergency_buffer.min((context_window / 16).max(1));

    request_ceiling
}
```

`default_output_reserve() = 20_000` (`compact.rs:95-97`) is **kept**. Making `output_reserve: Option<usize>` (a reviewer suggestion) would resolve an absent value to `window_output_unit(window)`, which at 128 k is 16 000 instead of 20 000 — i.e. it would *increase* the autocompact threshold from 95 000 to 99 000 and fill more window, the wrong direction for a fix about output truncation. Rejected on behavior, not aesthetics. `structural` is honestly described as a floor and a collapse target, not as "the derivation"; it is inert as a floor whenever `output_reserve` is at its default (because `window_output_unit ≤ 20_000` always).

### 3.3 Checked against `should_autocompact`

`autocompact_threshold` = `context_window.saturating_sub(output_reserve).saturating_sub(autocompact_buffer)` (`crates/agent/nomi-agent/src/compact/auto.rs:61-70`); `should_autocompact` fires at `last_input_tokens as usize >= threshold` (`:74`). Defaults 20 000 / 13 000 / 3 000 (`compact.rs:95-103`).

| window | declared | today's threshold | new threshold | delta |
|---|---|---|---|---|
| 200 k | 8192 | 200 000 − 20 000 − 13 000 = **167 000** | reserve max(20 000, 20 000, 8192)=20 000 → **167 000** | none |
| 200 k | **None** | *unrepresentable* | reserve max(20 000, 20 000, 0)=20 000 → **167 000** | none |
| 128 k | 8192 | **95 000** | reserve 20 000 → **95 000** | none |
| 128 k | **None** | *unrepresentable* | reserve 20 000 → **95 000** | none |
| 200 k | 64 000 | ceiling min(64 000, 50 000)=50 000; reserve 50 000 → **137 000** | identical → **137 000** | none (the first round's table wrongly claimed 123 000; `compact.rs:147-149` already caps at window/4) |
| 65 536 | 8192 | valve fires; reserve 8192, buf 8192 → **49 152** | valve; reserve max(8192, 8192)=8192, buf 8192 → **49 152** | none |
| 65 536 | **None** | *unrepresentable* | valve; reserve max(8192, 0)=8192, buf 8192 → **49 152** | none |
| 32 000 | 8192 | valve; reserve 8000, buf 4000 → **20 000** | valve; reserve max(4000, 8000)=8000, buf 4000 → **20 000** | none |
| 32 000 | **None** | *unrepresentable* | valve; reserve max(4000, 0)=4000, buf 4000 → **24 000** | +4 000 |
| 4096 | 8192 | ceiling 1024; valve; reserve 1024, buf 512 → **2 560** | identical → **2 560** | none |
| 4096 | **None** | *unrepresentable* | valve; reserve 512, buf 512 → **3 072** | +512 |

Neither failure mode is reachable:
- **Never-fires** would need `threshold ≥ context_window`, i.e. `reserve + buffer == 0`. `structural ≥ 1` by `(window/8).max(1)`, so `reserve ≥ 1` unconditionally. An explicit `output_reserve = 0` in TOML is now repaired to `structural` instead of being accidentally repaired to the ceiling.
- **Over-fires** would need `reserve + buffer ≥ context_window` (threshold saturates to 0, so every turn compacts). Non-valve branch: bounded by the branch condition at `≤ window/2`. Valve branch: `reserve = max(structural, permitted) ≤ window/4` and `buffer ≤ window/8`, so `≤ 3/8 window`. Threshold is always `≥ 5/8 window` after the valve and `≥ window/2` before it.

`autocompact_threshold_pct` (`compact.rs:48-49`) bypasses `output_reserve` entirely and is untouched.

All four existing tests at `compact.rs:333-383` pass with **only a `Some()` wrapper on the expected return** — recomputed above (rows 200 k/8192, 65 536/8192, 4096/8192, and the 32 000 custom 6000/4000/1000 case where `max(6000, 4000, 4096) = 6000`).

### 3.4 The compactor's own request

`crates/agent/nomi-agent/src/compact/auto.rs:126` becomes `max_tokens: Some(nomi_config::compact::window_output_unit(config.context_window))` and the import at `auto.rs:15-18` drops `compact_max_output_tokens`. This site deliberately does **not** intersect with the declared ceiling: `compact_max_output_tokens(200_000) = 20_000` is what it asks today, and the same endpoint that "could only do 8192" is already asked for 20 000 here — the standing proof that 8192 was never a provider constraint. Noting it as a known, pre-existing exemption rather than silently changing compaction behavior inside this workstream.

---

## 4. TRUE blast radius — verified counts

Every count below came from a script that opens each match and brace-balances the literal to decide whether it uses `..Default::default()`.

### `LlmRequest` — `max_tokens: u32 → Option<u32>` (`nomi-types/src/llm.rs:13`)
**8 production literals** (the first round listed 4):
| # | path:line (literal / field) | today |
|---|---|---|
| 1 | `crates/agent/nomi-agent/src/bootstrap.rs:38` / `:46` | `self.max_tokens` (field `:28`, wired `:816`) — **`#[cfg(feature = "browser-use")]`** |
| 2 | `crates/agent/nomi-agent/src/bootstrap.rs:99` / `:110` | same (field `:84`, wired `:829`) — **feature-gated** |
| 3 | `crates/agent/nomi-agent/src/compact/auto.rs:121` / `:126` | `compact_max_output_tokens(window)` |
| 4 | `crates/agent/nomi-agent/src/engine/mod.rs:1453` / `:1458` | `self.max_tokens` (field `:508`, set `:613`, `:692`) |
| 5 | `crates/backend/nomifun-ai-agent/src/factory/provider_config.rs:318` / `:323` | `max_tokens: u32` param `:313` → `Some(max_tokens)` |
| 6 | `crates/backend/nomifun-ai-agent/src/factory/provider_config.rs:351` / `:356` | param `:346` → `Some(max_tokens)` |
| 7 | `crates/backend/nomifun-ai-agent/src/image_generation.rs:162` / `:172` | `96` → `Some(96)` |
| 8 | `crates/backend/nomifun-ai-agent/src/one_shot.rs:126` / `:131` | `ONE_SHOT_MAX_TOKENS` → `Some(…)` |

5–8 keep their `u32` parameters, so their many callers (`nomifun-companion/src/learner.rs:110-113`, `nomifun-agent-execution/src/planner.rs:219,239`, `nomifun-creation/src/service.rs:1008`, `nomifun-app/src/services.rs:78`) are untouched.

**9 test literals** needing `Some(…)`: `nomi-providers/src/lib.rs:706`(`:711`); `openai.rs:3181`(`:3191`), `:3359`(`:3364`), `:3380`(`:3385`); `bedrock.rs:719`(`:745`), `:785`(`:795`); `tests/provider_anthropic_test.rs:21`(`:31`); `tests/provider_gemini_test.rs:12`(`:22`); `tests/provider_openai_test.rs:23`(`:33`). *`provider_anthropic_test.rs:106` and `provider_openai_test.rs:122` use a spread and are safe — the first round listed them as edit sites in error.*

**5 adapter readers**: `openai.rs:279`, `anthropic.rs:74`, `bedrock.rs:83`, `gemini.rs:159`, `vertex.rs:79`.

**7 assertion sites**: `openai.rs:3369` (unchanged, body JSON), `openai.rs:3391` (already `body.get("max_tokens").is_none()` — keep), `bedrock.rs:801` (unchanged), `provider_anthropic_test.rs:624` (unchanged), `provider_openai_test.rs:970` (unchanged), `crates/backend/nomifun-ai-agent/src/image_generation.rs:1537` → `Some(96)`, `crates/backend/nomifun-creation/src/service.rs:2319` → `Some(777)`.

### `TokenUsage` — `+ reasoning_tokens: u64` (`nomi-types/src/message.rs:117-124`)
**17 exhaustive literals in 7 files** (3 production). Derives `Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema` — the `JsonSchema` derive means an exported JSON schema changes too.
- **Production (3):** `crates/agent/nomi-providers/src/anthropic_shared.rs:741`, `crates/agent/nomi-providers/src/gemini.rs:787`, `crates/agent/nomi-providers/src/openai.rs:740`
- `crates/agent/nomi-types/src/llm.rs:87` (unit test)
- `crates/agent/nomi-agent/tests/common/mod.rs:39, :63`
- `crates/agent/nomi-agent/tests/engine_test.rs:685, 697, 923, 935, 991, 1036, 1048, 1168, 1180, 1233` (10)
- `crates/agent/nomi-agent/tests/json_stream_approval_test.rs:23`

The 15 spread literals in `tests/engine_compact_test.rs` (`:84,99,154,218,278,383,434,498,542,554,660,700,712,823,836`) and `tests/autocompact_test.rs:47` are genuinely unaffected. `cache_diagnostics.rs` contains **zero** `TokenUsage` literals — its ~17 literals are `CacheStats` (`cache_diagnostics.rs:19`); the first round named the wrong file.

### `NewProviderModelCapability` — `+ output_limit: Option<i64>` (`nomifun-db/src/models/provider_model.rs:61`)
38 literals total; **11 exhaustive in 6 files, 2 of them production**:
- **`crates/backend/nomifun-system/src/managed_model.rs:128` (PRODUCTION**, `context_limit: None` at `:140`)
- **`crates/backend/nomifun-system/src/provider_model.rs:613` (PRODUCTION**, `as_db()`, `context_limit` at `:625`)
- `crates/backend/nomifun-db/tests/provider_repository.rs:15, 31, 47, 63, 274, 927` (6; `:15` and `:927` are `static` arrays — `Option<i64>` keeps both `Copy` and const-constructibility)
- `crates/backend/nomifun-db/src/repository/sqlite_provider_connection.rs:247` (test `static`)
- `crates/backend/nomifun-model-invoke/src/service.rs:722` (test)
- `crates/backend/nomifun-shell/tests/stt_integration.rs:78`

The other 27 use `..Default::default()` and are safe.

### `ProviderModelCapabilityRow` — `+ output_limit: Option<i64>` (`provider_model.rs:38`)
**4 exhaustive literals, all in `#[cfg(test)]`**: `nomifun-agent-execution/src/participant_resolver.rs:761`, `nomifun-conversation/src/model_failover.rs:294`, `nomifun-conversation/src/service_test.rs:15556`, `nomifun-gateway/src/provider_support.rs:425`. (`model_failover.rs:282/:321` are function signatures, not literals.) Every read is `SELECT *` + `FromRow` (`sqlite_provider_model_capability.rs:220,232,246,262`), so no query edits.

### `ProviderModelCapabilityInput` — `+ output_limit` (`nomifun-api-types/src/provider_model.rs:134` twin)
**3 literals**: `nomifun-system/src/provider.rs:336` (production revalidation), `nomifun-system/src/provider_connection.rs:158` (production), `nomifun-system/src/provider_model.rs:785` (test).

### `NomiResolvedConfig` — `max_tokens: u32 → output_ceiling: Option<u32>` (`types.rs:124`)
**4 exhaustive literals**: `factory/nomi.rs:665` (prod), `services/provider_health.rs:150` (prod), `manager/nomi/agent.rs:3955` (test), `tests/agent_types_integration.rs:22` (test, field at `:28`).

### `ProtocolDescriptor` — `+ requires_output_ceiling: bool` (`model_protocol.rs:100`, no `Default`)
**3 literals**: `nomifun-model-invoke/src/manifest.rs:900` (`owned_protocol`, prod), `manifest.rs:1133` (`fake_descriptor`, test), `nomifun-model-invoke/tests/protocol_manifest.rs:55` (test). Plus one new assertion in `nomifun-api-types/tests/ts_export.rs` beside `:158-162`.

### `AgentInvocationInput` — **delete** `max_tokens: u32` (`nomi-types/src/agent.rs:397`)
**8 literals** (3 production, in **two crates**):
- **`crates/agent/nomi-skills/src/executor.rs:83` (PRODUCTION**, `max_tokens: 16384` at `:87`) — `nomi-skills` was absent from the entire first round
- `crates/agent/nomi-agent/src/local_delegate_tool.rs:100` (prod), `:157` (prod) — both `DEFAULT_AGENT_MAX_TOKENS`
- `crates/agent/nomi-agent/src/local_agent_invocation.rs:1055` (test), `:1069` (test, spreads), `:1615` (test)
- `crates/agent/nomi-agent/src/local_delegation_progress.rs:201` (test, `max_tokens: 1` at `:205`)
- `crates/agent/nomi-skills/src/executor.rs:510` and `crates/agent/nomi-agent/src/skill_tool.rs:971` are `fn take_input() -> AgentInvocationInput` signatures, not literals.

Consumed at `local_agent_invocation.rs:177 config.max_tokens = invocation.max_tokens`. Pinned by **TC-7.46** at `nomi-skills/src/executor.rs:732-742` (`assert_eq!(config.max_tokens, 16384)`) — must be rewritten, not just re-typed. `local_delegate_tool.rs:21-24 DESCRIPTION` advertises "*200 turns and 4096 output tokens each*" **to the model** and becomes a lie.

Decision: **delete**. All three producers hard-code a different invented number (4096 / 16384 / 1), a delegated Agent runs the same model over the same protocol as its parent so no fact distinguishes its ceiling, and 4096 is *smaller* than the parent's — a delegated agent truncates sooner for no reason, which is the very failure class being fixed. Subagents inherit `base_config.output_max_tokens` for free via `local_agent_invocation.rs:175 self.base_config.clone()`.

### `ResolvedTaskConfig` — `+ output_limit: Option<i64>` (`nomifun-model-invoke/src/call.rs:261`)
**3 literals**: `resolve.rs:444` (prod), `call.rs:460`, `call.rs:490` (tests).

### `TurnCompletedEventData` — `+ reasoning_tokens: u64`
**3 exhaustive literals**: `manager/nomi/agent.rs:1821` (prod), `:2142` (prod), `protocol/events/mod.rs:389` (test fixture, assertions `:397-402` and the back-compat case `:405-418`). The three in `nomifun-conversation/src/stream_relay.rs:7645, 7651, 7680` use `..Default::default()` and are safe.

### `Config` (nomi-config) — `max_tokens: u32 → output_max_tokens: Option<u32>` (`config.rs:482`)
**1 production literal** (`config.rs:646 max_tokens,`) and **13 test literals**: `nomi-agent/src/engine/compact_tests.rs:107`, `handle_command_tests.rs:31`, `phase6_tests.rs:34`, `plan_mode_tests.rs:38`, `set_config_tests.rs:1093`; `nomi-agent/tests/acceptance/helpers.rs:74, :110`; `tests/badcase_regression_test.rs:64`; `tests/bootstrap_test.rs:20`; `tests/common/mod.rs:249`; `tests/e2e/anthropic.rs:28`, `e2e/compaction.rs:36`, `e2e/openai.rs:27`.
**Readers**: `engine/mod.rs:613, :692`; `bootstrap.rs:816, :829`; `local_agent_invocation.rs:177`; `manager/nomi/agent.rs:60-61`.

### `ProviderError` — `+ Config(String)`
**Exactly one exhaustive match**: `redacted()` at `crates/agent/nomi-providers/src/lib.rs:96-121` (one new arm). `is_retryable` (`:124`), `retry.rs:65-71`, `retry.rs:74-86` all have `_ =>` catch-alls. (The first round's `ProviderError::Config` did not exist and nobody flagged it.)

### `build_request_body -> Result<Value, ProviderError>` call sites needing `?`
`anthropic.rs:152` **and `:175`** (the sanitize-and-retry second call); `bedrock.rs:275, :750, :799`; `vertex.rs:258`. `gemini.rs:118` already returns `Result` — the in-tree precedent.

### Existing tests that pin 8192 / the deleted fields (all will FAIL, not merely need re-typing)
1. `crates/agent/nomi-config/src/config.rs:1727` — `assert_eq!(merged.default.max_tokens, default_max_tokens())` (references the deleted fn)
2. `config.rs:2025` — `assert_eq!(config.default.max_tokens, 8192)`
3. `config.rs:2038/:2052` — TOML `max_tokens = 4096` → assert `4096` (now `Some(4096)`)
4. `config.rs:1663/:1673/:1684` — global 4096 / project 2048 → assert 2048 (the sentinel-comparison test; rewrite for `.or()`)
5. `config.rs:1699/:1713` — global 1024 / project default → assert 1024
6. `config.rs:2526/:2536/:2545` — TOML `max_tokens = 1234`, CliArgs `None` → `assert_eq!(config.max_tokens, 1234)` → `Some(1234)` on the renamed field
7. `config.rs:1796/:1817` — `ProfileConfig.max_tokens: Option<u32>` merge: **already `Option`, unchanged**
8. `crates/backend/nomifun-ai-agent/src/types.rs:294-302` `nomi_build_extra_serde_defaults` — asserts `8192`; **compile error** after deletion
9. `types.rs:304-315` `nomi_build_extra_serde_with_overrides` — asserts `4096`; **compile error**
10. `types.rs:317-326` `nomi_build_extra_serde_with_preset_rules` — passes `"max_tokens": 8192` in the JSON; drop the key
11. `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs:4453-4463` `provider_context_budget_is_applied_before_engine_bootstrap` — calls the deleted `apply_provider_context_budget`, sets `config.max_tokens = 8192` (`:4457`), asserts `config.max_tokens == 1024` (`:4462`) and `config.compact.output_reserve == 1024` (`:4463`)
12. `crates/agent/nomi-agent/tests/bootstrap_test.rs:308` — `assert_eq!(bootstrap.config().max_tokens, 1024)`
13. `crates/agent/nomi-skills/src/executor.rs:732-742` — TC-7.46
14. `crates/backend/nomifun-ai-agent/tests/factory_provider_integration.rs:239` — `extra: json!({ "max_tokens": 2048 })`. It survives only because `NomiBuildExtra` (`agent_build_extra.rs:112`) has no `deny_unknown_fields`, so it refutes §0's "zero producers"; remove the key.
15. `crates/agent/nomi-agent/src/compact/prompt.rs:214-219` — `compact_output_budget_scales_with_context_window` (moves to `compact.rs` as `window_output_unit_scales_with_context_window`)
16. `crates/agent/nomi-agent/tests/acceptance/compact_test.rs:130` — comment naming the deleted `COMPACT_MAX_OUTPUT_TOKENS`
17. `crates/agent/nomi-config/tests/compact_config_test.rs:15, 41, 61` — `output_reserve` 20 000 / 15 000: **unchanged by this design** ✓

### UI
- `ui/src/common/utils/providerModels.ts:67` — pass `output_limit` through (test: `providerModels.test.ts:98`)
- `ui/src/renderer/pages/settings/components/providerModelAdvanced.ts:55` (`ModelCapabilityDraft.outputLimit?: number`), `:122` (`emptyCapabilityDraft`), `:137` + `:153` (`capabilityDraftFromResponse`), `:641-643` (`capabilityInputFromDraft`), `:78-91` (`CapabilityValidationError` + `'output_ceiling_required'`)
- new `ui/src/renderer/pages/settings/components/OutputLimitInput.tsx` — an `InputNumber`, **not** a reuse of `ContextLimitSelect.tsx` (that is a preset picker because context windows cluster; output ceilings 4096/8192/16384/32768/64000/65536/128000 do not). Placeholder = `settings.outputLimitProviderDefault` when undefined, so "empty" reads as a deliberate state.
- `ui/src/renderer/pages/settings/components/ModelDefinitionEditor.tsx:1088-1096` — render it directly beneath the existing `ContextLimitSelect` block
- `ui/src/common/config/storage.ts:121` — **delete** dead `maxTokens?: number` (a repo-wide `grep maxTokens|max_tokens ui/src` returns only this line plus the unrelated `protocolBindings/TurnStopReason.ts:8`)
- `ui/src/renderer/services/i18n/*` + `i18n-keys.d.ts` (beside `:3384-3388`) — `settings.outputLimit`, `settings.outputLimitProviderDefault`, `settings.outputLimitRequired`
- Regenerated by ts-rs (do not hand-edit): `ProviderModelCapabilityInput.ts`, `ProviderModelCapabilityResponse.ts`, `ProtocolDescriptor.ts`, `TurnCompletedEventData.ts`
- Test literals: `ui/src/common/types/provider/providerApi.test.ts:32, :85`; `providerModelAdvanced.test.ts:290, 441, 465, 494, 508`; new `AddPlatformModal.outputLimit.test.ts` (source-assertion sibling of `AddPlatformModal.contextLimit.test.ts`)

---

## 5. Reasoning-token accounting — the minimal correct change

Today `update_stream_usage` (`crates/agent/nomi-providers/src/openai.rs:1323-1381`) parses `prompt_tokens`, `prompt_cache_hit_tokens`, `completion_tokens`, `prompt_tokens_details.cached_tokens` and the `normalizedUsage` branch (`:1326-1340`) — but **never `completion_tokens_details.reasoning_tokens`**. `step-3.7-flash` is a reasoning model (fixture at `openai.rs:2338`), so the whole diagnosis "thinking consumed the budget before any tool call" is invisible in the accounting.

1. `crates/agent/nomi-types/src/message.rs:117-124` — add
   ```rust
   /// Output tokens the provider attributed to internal reasoning. A SUBSET of
   /// `output_tokens`, never additional to it; 0 when the provider reports none.
   ///
   /// Anthropic extended thinking is already inside `output_tokens` and is not
   /// reported separately, so this stays 0 on the Anthropic family and is
   /// deliberately not synthesized there.
   #[serde(default)]
   pub reasoning_tokens: u64,
   ```
   `#[serde(default)]` matches the existing treatment of `cache_creation_tokens`/`cache_read_tokens` and keeps persisted `Session.total_usage` loadable.
2. **Simplify the 14 test literals instead of extending them.** `crates/agent/nomi-agent/tests/common/mod.rs:39,:63`, `engine_test.rs:685,697,923,935,991,1036,1048,1168,1180,1233`, `json_stream_approval_test.rs:23` all set `cache_creation_tokens: 0, cache_read_tokens: 0` — replace both lines with `..Default::default()`. Net line count falls and they stop breaking on the next field. `nomi-types/src/llm.rs:87` sets `cache_read_tokens: 5`, so it gets `reasoning_tokens: 0,`.
3. **The 3 production adapters** are the only sites that must set it meaningfully:
   - `openai.rs:740` (`drain_terminal_events`) — `reasoning_tokens: self.reasoning_tokens`. `StreamState` (`:642-667`) gains the field; `StreamState::new` (`:670-682`) initializes 0. The `pending_done` constructions at `openai.rs:1757, :1763, :1774` use `TokenUsage::default()` as a placeholder that `drain_terminal_events` replaces, so they need no edit.
   - `anthropic_shared.rs:741` — `reasoning_tokens: 0` with the doc reason above.
   - `gemini.rs:787` — `reasoning_tokens: 0` (Gemini reports `thoughtsTokenCount`; out of scope, stated).
4. `update_stream_usage` (`openai.rs:1323`) — parse `completion_tokens_details.reasoning_tokens` with the existing `optional_usage_u64` helper (`:1305`) and the same non-object rejection shape as `prompt_tokens_details` (`:1360-1371`); add `normalizedUsage.reasoningTokens` to the OpenRouter branch (`:1326-1340`).
5. `crates/agent/nomi-agent/src/engine/mod.rs:1825-1828` — add `self.total_usage.reasoning_tokens += turn_usage.reasoning_tokens;` beside the other three. `engine/mod.rs:428` (`StopReason::MaxTokens => "max_tokens"`) is a fixed dimension label in `terminal_dimensions` and is left alone.
6. `crates/backend/nomifun-ai-agent/src/protocol/events/mod.rs:104-120` — `#[serde(default)] #[ts(type = "number")] pub reasoning_tokens: u64`, populated at `manager/nomi/agent.rs:2142-2148` from `agent_result.usage.reasoning_tokens` (and `0` at `:1821-1827`, the image-capability path that already sends `input_tokens: 0`). Update the fixture at `:389-402`.

That is the whole change. **No** new `ModelTrait` (`ModelTrait::Reasoning` already exists at `nomifun-api-types/src/model_task.rs:55`), and **no** `reasoning_effort` step-down: `current_reasoning_effort` is initialized to `None` at `engine/mod.rs:627` and `:706`, and `set_initial_reasoning_effort` has exactly one caller in the tree (`local_agent_invocation.rs:154`, the subagent path). On the desktop chat path there is no level to step down from, so the first round's `>0.9` trigger would have fired and done nothing.

---

## 6. Recording an observed `finish_reason=length` — **DROPPED**

The first round's `provider_model_capabilities.output_observation` column, `OutputCeilingObservation` / `OutputTruncation` ts-rs types, `record_output_observation` repository method, `IProviderModelCapabilityRepository` trait method, the model-editor hint and the once-per-session inline notice are **all removed from this design**. Reasons, each checked:

1. **The row key is unreachable from the write site.** The `(provider_id, model, task)` key requires a repository handle and a `provider_id`. `NomiAgentManager` (`manager/nomi/agent.rs:108-206`) has **zero** repository/service fields — I listed every field; there is no repo, no `ModelInvokeService`, no `provider_id`, no `model`. `AgentRuntimeState` (`runtime_state.rs:37-48`) exposes only `conversation_id()` and `workspace()`. `NomiResolvedConfig` has `provider: String` — a **family** name, literally `"bedrock".to_owned()` at `factory/provider_config.rs:163` — and no `provider_id`. Making it reachable means adding a field to `NomiResolvedConfig`, an `Arc<dyn IProviderModelCapabilityRepository>` to `NomiAgentManager`, and threading it through `NomiAgentManager::new`. That is real plumbing this workstream does not need.
2. **The number already reaches the user on an existing surface.** A truncated turn is already `result_error_code = output_truncated`, `result_ok = 0` (`relay_error_code.rs:26,100,121-129` — A1, landed), and §5 puts `reasoning_tokens` on `TurnCompletedEventData`, so the UI token row reads *"output 24 576 (reasoning 23 904)"* on a turn already marked failed. A durable per-capability column is a second home for the same fact.
3. **Shipping it would be dead surface**, which the owner forbade.

If a durable learned bound is wanted later it belongs behind a writer that already owns a repo — `persist_probe_outcome` in `services/provider_health.rs:134-138` reaches `self.invoke.provider_model_capability_repo()`, and that is the shape to copy. Noted for D1, not built here.

Consequently **one** new column, not two. And I explicitly do **not** add a relay error code: `output_truncated` already exists and is already retryable. `fixed_lifecycle_codes_have_contracted_retryability` (`relay_error_code.rs:341-360`) already has its row at `:350`.

---

## 7. Ordered edit list

Bottom-up so the tree compiles at as many intermediate points as possible.

### Phase A — `nomi-types`
1. `crates/agent/nomi-types/src/llm.rs:13` — `pub max_tokens: Option<u32>` + the doc from §2.
2. `crates/agent/nomi-types/src/message.rs:117-124` — `+ reasoning_tokens: u64` (§5.1).
3. `crates/agent/nomi-types/src/llm.rs:87` — `+ reasoning_tokens: 0,`.
4. `crates/agent/nomi-types/src/agent.rs:397` — **delete** `pub max_tokens: u32`.

### Phase B — `nomi-config`
5. `crates/agent/nomi-config/src/compact.rs` — add `window_output_unit` (§3.2); rewrite `fit_context_budget` (`:137-170`) to the §3.2 signature; rewrite the `output_reserve` doc at `:13-16` to say plainly that this is input headroom, window-floored, never defined by the ceiling. Add `Some(...)` to the four expected returns at `:337, :350, :363, :379`. Add the `window_output_unit` scaling test moved down from `prompt.rs:214-219`, plus new rows for `declared = None` at 200 k / 128 k / 65 536 / 32 000 / 4096 asserting `output_reserve > 0` at every window.
6. `crates/agent/nomi-config/src/config.rs`:
   - `:510` — new `ProviderType::requires_output_ceiling(self) -> bool` (§2.2)
   - `:188-189` — `#[serde(default)] pub max_tokens: Option<u32>` (TOML key name unchanged — it is documented user surface)
   - `:200` — `max_tokens: None` in `impl Default for DefaultConfig`
   - `:447` — **delete** `default_max_tokens()`
   - `:482` — `pub output_max_tokens: Option<u32>` (rename + retype)
   - `:606` — `let output_max_tokens = cli.max_tokens.or(merged.default.max_tokens);`
   - `:646` — `output_max_tokens,` in the `Ok(Config { … })` literal
   - `:1049-1052` — the sentinel comparison **disappears**: `max_tokens: project.default.max_tokens.or(global.default.max_tokens),`
   - `:1309-1310` — `config.default.max_tokens = Some(max_tokens);`
   - `:1291` — already `overlay.max_tokens.or(base.max_tokens)`; unchanged
   - `:1365` — **delete** `max_tokens = 8192` from `DEFAULT_CONFIG_TEMPLATE`
   - tests `:1663, 1673, 1684, 1699, 1713, 1727, 2025, 2038, 2052, 2526, 2536, 2545` per §4

### Phase C — `nomi-providers`
7. `crates/agent/nomi-providers/src/lib.rs:43-50` — `OutputCeilingKey` + the new `request_body_with_extra` (§1a); `:60-90` — `+ ProviderError::Config(String)`; `:96-121` — one new `redacted()` arm.
8. `openai.rs:254-303` — conditional insert at `:279`; `OutputCeilingKey::Top(max_tokens_field)` at `:292`; no ceiling entry in the `object.remove` block.
9. `gemini.rs:155-184` — emit `generationConfig` only when non-empty; `OutputCeilingKey::Nested{…}` at `:174`.
10. `anthropic.rs:60` — `fn build_request_body(&self, request: &LlmRequest, sanitize_tool_schemas: bool) -> Result<Value, ProviderError>`; `:74` — `request.max_tokens.ok_or_else(|| ProviderError::Config("anthropic.messages requires an explicit output ceiling; declare “Max output tokens” on the model capability".into()))?`; `?` at `:152` and `:175`; `:103` gets the ceiling key.
11. `bedrock.rs:70` — same signature; `:83`; `?` at `:275, :750, :799`; `:112` ceiling key.
12. `vertex.rs:66` — same signature; `:79`; `?` at `:258`. No `request_body_with_extra` call, so no ceiling key.
13. `openai.rs:642-682, :740, :1323-1381` — reasoning-token accounting (§5.3-4).
14. `anthropic_shared.rs:741`, `gemini.rs:787` — `reasoning_tokens: 0` + doc.
15. Test literals per §4: `lib.rs:711`; `openai.rs:3191, 3364, 3385`; `bedrock.rs:745, 795`; `tests/provider_{anthropic,gemini,openai}_test.rs:31, 22, 33`.

### Phase D — `nomi-agent` + `nomi-skills`
16. `crates/agent/nomi-agent/src/engine/mod.rs:508, 613, 692, 1458` — `output_max_tokens: Option<u32>`; `:1825-1828` — reasoning accumulation.
17. `crates/agent/nomi-agent/src/compact/prompt.rs:10-21` — **delete** `COMPACT_MAX_OUTPUT_TOKENS` and `compact_max_output_tokens` and their test at `:214-219`; `crates/agent/nomi-agent/src/compact/auto.rs:15-18, :126` — import and call `nomi_config::compact::window_output_unit`, wrap in `Some`.
18. `crates/agent/nomi-agent/src/bootstrap.rs:28, 46, 84, 110, 816, 829` — `Option<u32>` (**feature-gated** — see the test plan).
19. `crates/agent/nomi-agent/src/local_delegate_tool.rs:19` — **delete** `DEFAULT_AGENT_MAX_TOKENS`; `:21-24` — rewrite `DESCRIPTION` to "*200 turns each, inheriting the session's output limit*"; `:100`, `:157` — drop the field.
20. `crates/agent/nomi-agent/src/local_agent_invocation.rs:177` — **delete** `config.max_tokens = invocation.max_tokens;`; literals `:1055, :1615`.
21. `crates/agent/nomi-agent/src/local_delegation_progress.rs:201-205` — drop the field.
22. `crates/agent/nomi-skills/src/executor.rs:83-95` — drop `max_tokens: 16384`; `:732-742` — rewrite TC-7.46 as `tc_7_46_fork_inherits_the_session_output_ceiling`, asserting `config.output_max_tokens == base.output_max_tokens`. (Keeping the numbered slot with a claim that is now true, per the no-vestigial rule.)
23. `crates/agent/nomi-cli/src/main.rs:66-68` — help text → `/// Max output tokens per response (omit to let the provider decide)`.
24. Config test literals per §4 (13 sites) + `tests/bootstrap_test.rs:308` + `tests/acceptance/compact_test.rs:130` comment.

### Phase E — persistence
25. **New** `crates/backend/nomifun-db/migrations/036_provider_model_output_limit.sql`:
   ```sql
   -- Declare the model's real output ceiling next to its context window.
   --
   -- Until now the only output ceiling was a single global default (8192) applied
   -- as `max_tokens` on every request regardless of model. That is not a fact any
   -- user can know globally: it is a per-model number published by the provider,
   -- exactly like context_limit. Declaring it here makes "undeclared"
   -- representable as NULL, which the request serializers translate into OMITTING
   -- the field so the provider applies its own maximum.
   --
   -- ADD COLUMN accepts a column-level CHECK; the constraint is column-local
   -- (see 020_channel_owner_domain.sql:10-15).

   ALTER TABLE provider_model_capabilities
       ADD COLUMN output_limit INTEGER
       CHECK (output_limit IS NULL OR output_limit > 0);

   -- Transcribe the status quo for the protocols whose body MANDATES the field.
   -- For anthropic.messages / bedrock.anthropic_messages, NULL is not a
   -- representable request state, so NULL is not a valid migrated value: it is a
   -- broken row, and a migration must not create broken rows. 8192 is not an
   -- invented number for these rows -- it is exactly what they put on the wire
   -- today via the global default. This moves that value into its new, editable
   -- home with zero behavior change and leaves no code-side fallback behind.
   --
   -- openai.chat_text and gemini.generate_text are deliberately NOT backfilled:
   -- for them NULL is representable and omission IS the fix.
   UPDATE provider_model_capabilities
      SET output_limit = 8192
    WHERE task = 'chat'
      AND protocol IN ('anthropic.messages', 'bedrock.anthropic_messages');
   ```
   **Migration-target decision, stated as `docs/contributing/data-and-identifier-standards.md:49-53` requires:** an append migration is the correct target, not an in-place amendment of `032_provider_model_capabilities.sql:71`. `crates/backend/nomifun-db/src/database.rs:31-40` documents that a **checksum mismatch** in the persisted lineage "is unsupported and must fail closed before writable startup", so amending 032 (which has shipped — 033/034/035 land after it) would brick every existing install. 036 is the next free number; `static DB_MIGRATOR: Migrator = sqlx::migrate!();` (`database.rs:28`) auto-discovers it. The one-time backfill statements *inside* 032 (`:158, :323, :378`) operate on the retired `provider_models` table and are frozen.
26. `crates/backend/nomifun-db/src/models/provider_model.rs:38` — `+ pub output_limit: Option<i64>` on `ProviderModelCapabilityRow`; `:61` — same on `NewProviderModelCapability` (keeps `Copy` and const-constructibility for the `static` arrays).
27. `crates/backend/nomifun-db/src/repository/sqlite_provider_model_capability.rs` — add `output_limit` to the INSERT column list (`:108`), the `DO UPDATE SET` (`:123`), the `WHERE NOT (…)` change-detection predicate (`:137`), and the `.bind()` chain (`:153`). `health = NULL` / `health_checked_at = NULL` at `:111-112` stay as they are.
28. `crates/backend/nomifun-db/src/repository/sqlite_provider.rs:374-383` — add `output_limit` to both lists of the provider-**clone** `INSERT … SELECT`. (A clone points at a *different* connection but the same model, and the declared ceiling is a property of the model, not the connection — so it *is* copied. Stated explicitly because the `INSERT … SELECT` shape makes an omission invisible.)
29. Literals per §4: `provider_repository.rs:15, 31, 47, 63, 274, 927`; `sqlite_provider_connection.rs:247`; `nomifun-model-invoke/src/service.rs:722`; `nomifun-shell/tests/stt_integration.rs:78`; `participant_resolver.rs:761`; `model_failover.rs:294`; `service_test.rs:15556`; `provider_support.rs:425`.

### Phase F — API types, manifest, validation
30. `crates/backend/nomifun-api-types/src/provider_model.rs:134` twin — `#[serde(default)] #[ts(optional, type = "number")] pub output_limit: Option<i64>` on `ProviderModelCapabilityInput`; `:197` twin — same on `ProviderModelCapabilityResponse` with `#[serde(skip_serializing_if = "Option::is_none")]`.
31. `crates/backend/nomifun-api-types/src/model_protocol.rs:100-116` — `+ pub requires_output_ceiling: bool` on `ProtocolDescriptor`.
32. `crates/backend/nomifun-api-types/src/agent_build_extra.rs:119-120, :231-233` — **delete** `NomiBuildExtra.max_tokens` and `default_nomi_max_tokens()`.
33. `crates/backend/nomifun-api-types/tests/ts_export.rs` — `output_limit?: number,` beside the `context_limit?: number,` assertion at `:121`, and `requires_output_ceiling: boolean` beside the `ProtocolDescriptor` block at `:158-162`.
34. `crates/backend/nomifun-model-invoke/src/manifest.rs:424` — `protocol_requires_output_ceiling` (§2.5); `:900` — populate the descriptor field; `:750` — the `OUTPUT_CEILING_PARAM_KEYS` rejection (§1b); literals `:1133`, `tests/protocol_manifest.rs:55`. Export at `crates/backend/nomifun-model-invoke/src/lib.rs:47`.
35. `crates/backend/nomifun-model-invoke/src/call.rs:261` — `+ pub output_limit: Option<i64>` on `ResolvedTaskConfig`; literals `:460`, `:490`.
36. `crates/backend/nomifun-model-invoke/src/resolve.rs:438-441` — extend the `<= 0` guard to `output_limit`; `:444-457` — populate the field.
37. `crates/backend/nomifun-system/src/provider_model.rs:235-242` — rename `validate_context_limit` → `validate_positive_token_limit(name: &str, value: Option<i64>)`; call twice at `:182`; add the required-ceiling check there (`protocol_requires_output_ceiling(&capability.protocol) && capability.output_limit.is_none()` → `AppError::BadRequest`), the backend-authoritative twin of the UI marker. `:26-27` + `:453` in `provider.rs` follow the rename.
38. `crates/backend/nomifun-system/src/provider_model.rs:608` (`SerializedCapability`), `:625` (`as_db`), `:656` (`serialize_capabilities`), `:772` (row→response), `:797` (test fixture) — thread `output_limit`.
39. `crates/backend/nomifun-system/src/provider.rs:348`, `crates/backend/nomifun-system/src/provider_connection.rs:170` — thread it through the response→input revalidation literals.
40. `crates/backend/nomifun-system/src/managed_model.rs:140` — `output_limit: None` (production literal).
41. `crates/backend/nomifun-gateway/src/caps_system.rs:153` — `+ output_limit: Option<i64>` on the MCP capability params, else the gateway cannot author what the UI can.

### Phase G — host wiring
42. `crates/backend/nomifun-ai-agent/src/factory/provider_config.rs:44` — `+ pub output_limit: Option<i64>` on `ResolvedProviderFields`; `:218` — `output_limit: task.output_limit` (the exact `context_limit` twin). **No gate here** (§2.1).
43. `crates/backend/nomifun-ai-agent/src/types.rs:123-124` — `max_tokens: u32` → `output_ceiling: Option<u32>` (§2.4); tests `:294-326`.
44. `crates/backend/nomifun-ai-agent/src/factory/nomi.rs:671` — `output_ceiling: fields.output_limit.map(|v| v as u32)`.
45. `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs:55-62` — `apply_provider_token_budget` (§2.3); `:948` — `max_tokens: config_extra.output_ceiling`; `:984` — `apply_provider_token_budget(&mut config, config_extra.context_limit)?`; `:3961` and `:4442/:4453-4463` tests.
46. `crates/backend/nomifun-ai-agent/src/services/provider_health.rs:159` — `output_ceiling: Some(16)`; `:397` — `max_tokens: config_extra.output_ceiling`; `:768-769` test literal; `:150` `NomiResolvedConfig` literal.
47. `crates/backend/nomifun-ai-agent/src/factory/provider_config.rs:323, :356`; `image_generation.rs:172` + `:1537`; `one_shot.rs:131` — `Some(...)`.
48. `crates/backend/nomifun-ai-agent/src/protocol/events/mod.rs:104-120` — `reasoning_tokens`; `:389-418` fixture; `manager/nomi/agent.rs:1821-1827, :2142-2148`.
49. `crates/backend/nomifun-creation/src/service.rs:2319` — `Some(777)`.
50. `crates/backend/nomifun-ai-agent/tests/agent_types_integration.rs:22-28`; `tests/factory_provider_integration.rs:239`.

### Phase H — UI
51–56. Per §4's UI block, in that order (regenerate bindings first).

---

## 8. OLD CODE DELETED — no alias, no fallback, no dual read

| what | where |
|---|---|
| `NomiBuildExtra.max_tokens: u32` | `crates/backend/nomifun-api-types/src/agent_build_extra.rs:119-120` |
| `default_nomi_max_tokens() -> 8192` | `crates/backend/nomifun-api-types/src/agent_build_extra.rs:231-233` |
| dead `maxTokens?: number` | `ui/src/common/config/storage.ts:121` |
| `default_max_tokens() -> 8192` | `crates/agent/nomi-config/src/config.rs:447` |
| `max_tokens = 8192` in `DEFAULT_CONFIG_TEMPLATE` | `crates/agent/nomi-config/src/config.rs:1365` |
| the "is it the built-in default? then inherit" sentinel comparison | `crates/agent/nomi-config/src/config.rs:1049-1052` |
| `NomiResolvedConfig.max_tokens: u32` | `crates/backend/nomifun-ai-agent/src/types.rs:124` |
| `apply_provider_context_budget` | `crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs:55-62` |
| `config.output_reserve = max_tokens as usize` — the coupling itself | `crates/agent/nomi-config/src/compact.rs:152, :159` |
| unconditional `body[max_tokens_field] = json!(request.max_tokens)` | `crates/agent/nomi-providers/src/openai.rs:279` |
| `COMPACT_MAX_OUTPUT_TOKENS = 20_000` + `compact_max_output_tokens` (moved down, not duplicated) | `crates/agent/nomi-agent/src/compact/prompt.rs:11, :16-21` |
| `DEFAULT_AGENT_MAX_TOKENS: u32 = 4096` | `crates/agent/nomi-agent/src/local_delegate_tool.rs:19` |
| "(200 turns and 4096 output tokens each)" in the model-facing tool description | `crates/agent/nomi-agent/src/local_delegate_tool.rs:21-24` |
| `AgentInvocationInput.max_tokens: u32` | `crates/agent/nomi-types/src/agent.rs:397` |
| `max_tokens: 16384` (the third invented number) | `crates/agent/nomi-skills/src/executor.rs:87` |
| `config.max_tokens = invocation.max_tokens` | `crates/agent/nomi-agent/src/local_agent_invocation.rs:177` |
| TC-7.46's `assert_eq!(config.max_tokens, 16384)` | `crates/agent/nomi-skills/src/executor.rs:732-742` |
| `nomi_build_extra_serde_defaults` / `..._with_overrides` | `crates/backend/nomifun-ai-agent/src/types.rs:294-315` |

**Also deleted from the first-round design (not from the repo):** `OutputCeilingPolicy` + the `LlmProvider` trait method + the `SecretRedactingProvider` forwarding + its test; `NomiResolvedConfig.request_output_ceiling`; `effective_ceiling`; the `output_observation` column, `OutputCeilingObservation`, `OutputTruncation`, `record_output_observation`, the repository trait method, the `output_ceiling_notice_shown` session key; the `output_token_ceiling` relay error code.

Nothing above gains a `#[serde(rename)]`, alias, compat shim, or dual-read path. The only data written by the migration is the value those rows already put on the wire, and no code reads a default after it.

---

## 9. Test plan

Narrow commands, in the order they should first pass.

```
cargo test -p nomi-types
cargo test -p nomi-config                      # compact.rs table test + compact_config_test.rs
cargo test -p nomi-providers                   # 5 adapters, extra_body strip, redacted()
cargo test -p nomi-agent --features browser-use   # REQUIRED: bootstrap.rs:28,46,84,110,816,829
                                                  # are behind `browser-use` (nomi-agent/Cargo.toml:16),
                                                  # so a default `cargo test` never compile-checks them
cargo test -p nomi-skills executor
cargo test -p nomi-cli
cargo test -p nomifun-db --test provider_capabilities_migration
cargo test -p nomifun-db --test provider_repository
cargo test -p nomifun-api-types --test ts_export
cargo test -p nomifun-model-invoke
cargo test -p nomifun-system provider_model
cargo test -p nomifun-ai-agent
cargo test -p nomifun-ai-agent --test provider_config_protocol_contract
cargo test -p nomifun-ai-agent --test factory_provider_integration
cargo test -p nomifun-ai-agent --test agent_types_integration
cargo test -p nomifun-conversation relay_error_code
cargo test -p nomifun-gateway provider_support
cargo test -p nomifun-shell --test stt_integration
cargo test -p nomifun-creation service
cargo test -p nomifun-agent-execution participant_resolver
cargo test -p nomifun-knowledge --test retrieval_pipeline
bun test ui/src/renderer/pages/settings/components/AddPlatformModal.outputLimit.test.ts
bun test ui/src/renderer/pages/settings/components/providerModelAdvanced.test.ts
bun test ui/src/common/utils/providerModels.test.ts
bun run typecheck
```

New tests that pin the design:

1. **`crates/agent/nomi-agent/tests/badcase_regression_test.rs`** — the house home for "this exact failure must never recur". Real engine + real `openai` adapter over `scripted_server` (`:127`) with the `RecordingResponder` (`:106`) that captures request bodies.
   - `Config.output_max_tokens = None` ⇒ the captured body has **no `max_tokens` key at all**, and specifically not `"max_tokens": null`. Highest-value single assertion in the change.
   - Same, with `compat.extra_body = {"max_tokens": 32000}` ⇒ still **absent**. This is the blocker-3 pin, and it directly contradicts the pre-change behavior proven by `tests/provider_openai_test.rs:953/:970`.
   - `Config.output_max_tokens = Some(2048)` with `extra_body = {"max_tokens": 1}` ⇒ `body["max_tokens"] == 2048` (the existing guarantee still holds).
   - `max_tokens_field = "max_completion_tokens"` + `extra_body = {"max_completion_tokens": 1}` + `None` ⇒ neither key present.
   - A scripted `finish_reason: "length"` with `completion_tokens_details.reasoning_tokens: 23904` ⇒ `AgentResult.usage.reasoning_tokens == 23904`, `stop_reason == StopReason::MaxTokens`.
2. **`crates/agent/nomi-providers/tests/provider_anthropic_test.rs`** (+ bedrock, vertex in-file tests) — `max_tokens: None` ⇒ `ProviderError::Config`. Never a body without the field, never an invented number. Explicitly assert the error is **not** `unwrap_or(8192)` or `unwrap_or(window/8)`: Claude 3.5 Sonnet is a 200 k window with an 8192 output max, so `window/8 = 25 000` would 400.
3. **`crates/agent/nomi-config/src/compact.rs`** — the §3.3 table verbatim, including every `declared = None` row, plus `assert!(cfg.output_reserve > 0)` for every window with `None`, plus `output_reserve = 0` in TOML ⇒ repaired to `window_output_unit(window)`.
4. **`crates/backend/nomifun-ai-agent/tests/provider_config_protocol_contract.rs`** — the cross-crate pin for the two unlinkable sources of truth. The file already resolves all four Chat protocols at `:179, :199, :216, :233` and gets a `Config` back, hence a `ProviderType`:
   ```rust
   assert_eq!(
       config.provider.requires_output_ceiling(),
       nomifun_model_invoke::protocol_requires_output_ceiling(protocol),
       "protocol {protocol} disagrees with its adapter about the output ceiling"
   );
   ```
   `nomifun-ai-agent` is the only crate that depends on both `nomi-config` and `nomifun-model-invoke` (`Cargo.toml:48` and its model-invoke dep), so this is the only place the check can live. Without it a 5th Chat protocol routed to the anthropic adapter would pass authoring and the host gate and hard-error mid-turn.
5. **`crates/backend/nomifun-ai-agent/src/manager/nomi/agent.rs`** — replace `provider_context_budget_is_applied_before_engine_bootstrap` (`:4453-4463`) with three cases: (a) declared 8192 at a 4096 window ⇒ `Some(1024)`, reserve 1024; (b) declared `None` at 200 k on `ProviderType::OpenAI` ⇒ `Ok`, `output_max_tokens == None`, reserve 20 000; (c) declared `None` on `ProviderType::Anthropic` ⇒ `Err(AppError::BadRequest)` whose message contains `"Max output tokens"`.
6. **`crates/backend/nomifun-ai-agent/src/services/provider_health.rs`** — an Anthropic-protocol probe with `output_limit = NULL` on the row still succeeds (`output_ceiling: Some(16)` reaches the wire via `CliArgs`). This is the blocker-4 regression pin.
7. **`crates/backend/nomifun-db/tests/provider_capabilities_migration.rs`** — 036 applies over 032/033/034/035; `output_limit` is `NULL` for pre-existing `openai.chat_text` and `gemini.generate_text` rows and `8192` for `anthropic.messages` / `bedrock.anthropic_messages` chat rows; the `CHECK` rejects `0` and `-1`; a full capability save round-trips the value; the retired-column loop at `:289` targets `PRAGMA table_info(provider_models)` and is unaffected.
8. **`crates/backend/nomifun-model-invoke`** — `validate_provider_params_for_protocol` rejects `max_tokens`, `max_completion_tokens`, `maxOutputTokens`, `max_output_tokens` and a matching `max_tokens_field` name on `Chat`, and still accepts them on non-Chat tasks (`nomifun-model-invoke/src/adapters/siliconflow.rs:161,215` legitimately uses `max_tokens` for TTS).
9. **`ui/.../AddPlatformModal.outputLimit.test.ts`** — source-assertion sibling of `AddPlatformModal.contextLimit.test.ts`: `ModelDefinitionEditor.tsx` contains `<OutputLimitInput` and `value={capability.outputLimit}`; `providerModelAdvanced.ts` contains `output_limit: capability.outputLimit`; `OutputLimitInput.tsx` contains `settings.outputLimitProviderDefault`.

Checks I cannot run here and that must be run before landing: the whole list above, plus `bun run test:ui` (which has one known pre-existing failure from an upstream `CreateStudio` modal restyle) and the `nomifun-app` lib suite (which flakes exactly one rotating loopback test per run at `.send()`).

---

## 10. How this design behaves on the observed failure

Provider `openai-compatible`, model `step-3.7-flash` (StepFun), a reasoning model. Medium coding task, 2 turns, every model pass `stop_reason=max_tokens` with zero tool calls, `output_tokens = 24 576 = 3 × 8192`, receipt `result_ok=1 result_error=NULL`, nothing on disk.

**Before, step by step.** `NomiBuildExtra.max_tokens` defaults to 8192 (`agent_build_extra.rs:231`) → `factory/nomi.rs:671` → `NomiResolvedConfig.max_tokens` → `agent.rs:948 CliArgs.max_tokens = Some(8192)` → `Config::resolve` → `agent.rs:984 fit_context_budget(compact, 8192)` returns 8192 → engine field (`engine/mod.rs:613`) → `LlmRequest.max_tokens = 8192` (`:1458`) → `openai.rs:279 body["max_tokens"] = 8192`. The model spent all 8192 on reasoning, emitted no tool call, `finish_reason=length` → `StopReason::MaxTokens`. `agent.rs:2057` auto-continued twice, each pass again capped at 8192, then emitted `Finish`. At the time of the failure `turn_succeeded` did not consult the stop reason, so a long prose stream read as `result_ok=1`.

**After, step by step.**

1. **Turn 1, first pass.** The `step-3.7-flash` chat row has `output_limit = NULL` (openai.chat_text is not backfilled — omission is the fix, not the bug). `factory/nomi.rs:671` sets `output_ceiling: None`; `agent.rs:948` sets `CliArgs.max_tokens: None`; `Config::resolve` at `config.rs:606` finds no `[default] max_tokens` either, so `Config.output_max_tokens = None`.
2. **Pre-turn gate.** `apply_provider_token_budget` at `agent.rs:984`: `fit_context_budget(compact, None)` returns `None` and sets `output_reserve = max(20 000, window_output_unit(200 000)=20 000, 0) = 20 000` — the autocompact threshold stays at **167 000, bit-for-bit unchanged**. `config.provider` is `ProviderType::OpenAI`, so `requires_output_ceiling()` is `false` and the turn proceeds. No error, no invented number.
3. **On the wire.** `openai.rs:279` inserts nothing. `request_body_with_extra` strips `max_tokens` from `extra_body` before the merge, so the key is **absent entirely** — not `null`, and not a smuggled `provider_params` value. StepFun now applies its own maximum instead of the 8192 nobody chose.
4. **What changes.** The model no longer hits a ceiling it never had. Either it finishes the tool call it was reaching for, or it hits StepFun's real maximum — a genuine model fact rather than a host-side guess.
5. **If it still truncates,** `finish_reason=length` → `StopReason::MaxTokens` → `map_engine_stop_reason` (`agent.rs:778`) → `TurnStopReason::MaxTokens` → `incomplete_stop_code` (`relay_error_code.rs:100`) → `result_error_code = "output_truncated"`, `result_error_retryable = true`, `result_ok = 0`. `turn_succeeded` (`:121-129`) cannot return true however much prose streamed, and a committed artifact cannot launder it (`:253-256`). **The false `result_ok=1` is closed** — by A1, which has landed, not by anything this design adds; this workstream deliberately opens no second false-success path.
6. **The diagnosis is now legible.** `TurnCompletedEventData` carries `output_tokens: 24 576, reasoning_tokens: 23 904`, so the UI token row reads *"output 24 576 (reasoning 23 904)"*. That converts *"the model wrote 24 576 tokens of prose"* into *"the model spent its entire budget thinking and emitted nothing"* — a different diagnosis with a different remedy, and the number the user needs before touching any setting.
7. **The user can now act.** Settings → Models → `step-3.7-flash` → chat capability → **Max output tokens**. Typing StepFun's published number makes the ceiling a declared fact; leaving it empty reads as *"provider default (field omitted)"* rather than a missing entry. The one thing they cannot do is put `max_tokens` in Provider params — that is rejected at save with a message naming the real control.
8. **What this design does NOT claim.** It does not by itself stop a reasoning model from thinking until any budget is gone. Removing an arbitrary 8192 makes the pass *able* to finish; it does not force a tool call. Recovery and the resumable round belong to **B1** (`agent.rs:2052-2087`), which owns `MAX_TRUNCATION_AUTO_CONTINUES` and the continuation prompt at `:720-733`. C1's contribution to that recovery is that after this change `output_truncated` is a *true* signal about a *real* provider limit instead of a host-invented one, so B1's retry has something honest to key on. **C1 must not land before A1** (it has) and should land alongside or before B1.

**Anthropic-family regression check on the same change.** An existing install with an `anthropic.messages` capability: 036 backfills `output_limit = 8192`, exactly what that row puts on the wire today. `factory/nomi.rs:671` → `output_ceiling: Some(8192)` → `fit_context_budget` → `Some(8192)` → `anthropic.rs:74` sends `"max_tokens": 8192`. Byte-identical body, `requires_output_ceiling` never fires, the health probe is untouched (`Some(16)` through its own `CliArgs`), and all four `provider_config_protocol_contract.rs` cases still resolve. The only way to reach the pre-turn `BadRequest` is a row authored *after* this change without the field — via the MCP gateway or a direct API call — which is precisely what the gate is for, and it fails before one token is spent with a message naming the control.