# Model providers and task routing

Current code owns configuration and invocation contracts. Vendor catalogs, account permissions and endpoint availability change; historical verification matrices remain in Git, outside the runtime capability registry and permanent allowlists.

## One configuration source

| Data | Ownership |
| --- | --- |
| `providers` | Provider identity, enabled state, configuration revision and default connection |
| `provider_connections` | Named connection URLs, authentication and encrypted credentials |
| `provider_models` | Model IDs, display, ordering and enabled state |
| `provider_model_capabilities` | Exact `(provider_id, model, task)` protocol, connection role, transport, parameters, limits and health |

Model IDs remain manually editable when discovery fails. Only provider-declared tasks or an exact official-document profile may automatically suggest interfaces. Model names and the existence of a provider adapter do not prove that a particular model supports a task. Background refresh never rewrites saved interfaces. Input understanding belongs to Chat and grants no generation or speech route.

`ALL_MODEL_TASKS` currently declares Chat, realtime conversation, image generation, image editing, video generation, music generation, speech synthesis, speech recognition, Embedding and Rerank. A model may configure several interfaces; each invocation resolves one exact task. Missing tasks return `UnsupportedTask`. Unknown protocols, incompatible tasks and missing adapters fail explicitly without provider-name inference or an OpenAI fallback.

## Protocol and connection boundaries

The `nomifun-model-invoke` protocol registry owns serializers, supported tasks, authentication, endpoint defaults and editability. Editors read its `/api/model-protocols` manifest rather than maintaining another protocol list. Configuration suggestions affect execution only after explicitly saving the capability; runtime does not repeat recommendation or infer protocols from model names.

Tasks under one provider may use different domains, named connections and credentials. `base_url_override` affects only the selected task; `endpoint`, `poll_endpoint`, `content_endpoint` and `realtime_endpoint` represent submission, polling, content and realtime transport. Absolute overrides with a different credential origin require explicit `allow_cross_origin_credentials`. Changing protocols clears old transport overrides and cross-origin consent before applying the new manifest. Authored parameter drafts and limits remain intact; incompatible protocol controls surface explicit editor and save-time errors for the user to correct.

`provider_params` obeys the protocol's strict encoding contract and cannot carry credentials or local transport fields. Values that cannot be encoded losslessly are rejected on save. Responses expose `has_credentials` without returning secrets. Invocation-graph changes atomically increment `config_revision`; resolution, asynchronous jobs and health observations use a consistent revision, so stale probes cannot overwrite new configuration.

Chat suggestions come from the [official-document profiles](../../crates/backend/nomifun-model-invoke/src/model_profiles.rs),
with generic OpenAI-compatible system/manual templates defaulting to `openai.chat_text`.
OpenAI, MiniMax and xAI retain their documented Responses defaults. Responses support alone
does not promote a template. Only defaults and their parameters change: automatic checking,
automatic filling and explicit user choices keep their existing behavior. The NomiFun gateway
retains the catalog's exact protocol declarations. Saved protocols are never rewritten automatically.
Responses uses its own typed serializers for input, function calls and stream events.
Tool schemas set `strict: false` explicitly to retain optional platform arguments.
Reasoning summary parts are validated separately by item and summary/content index.

`max_tokens_field` selects only the output field alias for either OpenAI Chat
protocol. The smaller of the typed request limit and capability `output_limit`
remains authoritative; aliases cannot replace messages, tools or protocol state.
Chat Completions accepts `stream_options: null` to omit optional usage options,
or an object whose `include_usage` overrides the default. Responses `include`
arrays merge with encrypted reasoning continuation fields; `include: null`
omits that option for compatible endpoints rejecting it. Missing encrypted
continuation is never fabricated.
Parameter JSON cannot inject protocol-owned model, input, tool, streaming or
retained cloud-state fields.
Reasoning effort uses the normalized `reasoning_effort` control. Saving native
Responses `reasoning.effort` asks the user to use that control instead, while
other native options such as `summary`, `mode` and `context` remain configurable.
Chat Completions omits local round IDs, tool metadata and `is_error`. Tool errors
are expressed in result text; images and audio follow the complete tool-result
batch as user attachments, keeping the tool messages themselves text-only.

“OpenAI compatible” does not prove compatibility across modalities. Ark versus Volcengine voice, native DashScope and MiniMax regional connections are expressed by their adapters and manifests, without a duplicate endpoint matrix. Provider changes update the registry, serializer and focused contract tests, then require actual-account verification. Local substitutes do not prove cloud permissions or model experience.

## Documented recommendations and catalogs

Active advisory data is split into [global native platforms](../../crates/backend/nomifun-model-invoke/src/model_profiles/global.json),
[China model vendors](../../crates/backend/nomifun-model-invoke/src/model_profiles/china-foundation.json),
[cloud platforms and plans](../../crates/backend/nomifun-model-invoke/src/model_profiles/cloud-platforms.json),
and [gateways and other services](../../crates/backend/nomifun-model-invoke/src/model_profiles/gateways.json).
Every entry records official sources, its review date, protocol evidence and
unknowns. These profiles supply configuration suggestions, not a runtime allowlist.

`model_defaults` returns only verified exact-model context, recommended output,
reasoning levels and protocol parameters. Native catalog values take priority;
documents fill missing fields. An output recommendation can use a documented
synchronous/default ceiling rather than a batch-only, background-only, or
full-context theoretical maximum. Gemini 4's combined window is distinguished
from earlier models' independent input ceilings. Saved values, explicit edits
and explicit provider-default choices remain authoritative. Unknown model IDs
remain editable without inheriting guessed family or upstream-vendor limits.

Model, protocol and URL recommendations travel together: some Zhipu Responses
models use `/api/v1`, while Alibaba Coding Plan retains Chat Completions.
Bedrock's new Mantle endpoints are not advertised as existing InvokeModel routes.
Aggregator limits and availability are never inferred from an upstream model name.

Task/protocol evidence is recorded separately for [global platforms](../../crates/backend/nomifun-model-invoke/src/provider_protocol_audit/global-and-gateways.json),
[Chinese platforms, plans and voice](../../crates/backend/nomifun-model-invoke/src/provider_protocol_audit/china-and-voice.json),
and [aggregators](../../crates/backend/nomifun-model-invoke/src/provider_protocol_audit/gateways.json).
Each group covers all ten tasks with supported, catalog-required, unimplemented,
unverified or retired status. Restrictions are specific to the platform, region
and plan, without inheriting an upstream model's capabilities. Added native
contracts include Gemini Interactions images/TTS and batch embeddings,
MiniMax/Zhipu/SiliconFlow ASR, Zhipu TTS, and InfiniAI rerank. Current Volcano voice auth and
URL-only ASR input, and several vendor video/music/realtime surfaces, still
require dedicated implementations. A similarly named OpenAI serializer is not
a working default. Unknown catalog IDs without declared tasks stay unconfigured
rather than automatically acquiring Chat.

## Actual protocol verification during setup

After model selection is committed, the editor calls
`/api/providers/discover-model-protocols` or
`/api/providers/{provider_id}/discover-model-protocols` with the exact model and
capability draft. Chat checks reuse production OpenAI, Responses, Anthropic,
Gemini and Bedrock providers, validating a streamed tool call, its real constant
result, and a final visible answer. They create no Session and run no user tools.
Each candidate has output/time bounds; production HTTP may perform finite retries.
All candidates retain their outcome. An available saved/manual choice wins,
followed by the verified documented recommendation, then the first available
candidate. Global authentication, quota and rate-limit failures stop further calls.

Media candidates use the [vendor protocol audit](../../crates/backend/nomifun-model-invoke/src/provider_protocol_audit.rs)
with exact documented model selectors or native catalog task declarations. The
codec, root, endpoints and native parameters travel together. A registered
serializer alone does not establish vendor support, and generic catalog task
labels cannot bypass per-model protocol restrictions. Evidence distinguishes live
verification, documented models, catalog declarations and unverified serializers.
Automatic checks submit no image, video, music or other generation jobs.

Discovery writes no catalog and grants no runtime protocol fallback. Saved
providers use encrypted credentials and revision fences; new providers use the
current form connections. Vendor defaults never replace custom relay roots.
Documented regional connections can select native root paths while preserving
the configured region and credential origin.
Catalog selection, Enter or blur commits an ID; a 300ms debounce and editor-local
cache prevent duplicate checks. Parameter/credential changes cancel stale requests,
while alias edits do not recheck. Applying results checks the exact draft snapshot,
and saving waits for every pending task. The normal view shows only the connection result. Available protocols are
inside Connection settings, with raw alternative failures behind Technical diagnostics.
Optional null numeric values mean unset rather than an invalid limit; genuine
input errors belong beside the relevant field.
Saved or user-edited parameters send `apply_recommended_defaults: false`, so
checks cannot restore explicitly deleted fields. Recommendations fill only new,
unmodified drafts. Automatically owned values are released when the model ID
changes, while persisted/manual values remain authoritative.
Chat/Responses parameter conversion is verified against shared frontend/backend
test vectors. It maps `response_format` and `text.format` using the
[official OpenAI migration shapes](https://developers.openai.com/api/docs/guides/migrate-to-responses),
and preserves verbosity and reasoning semantics. Local effort configuration remains
the normalized top-level `reasoning_effort`. Unknown vendor extensions retain their
original values; their names cannot establish protocol support.
Manual switching holds protocol-specific controls in the current editor draft so
switching back can restore them. This UI metadata never enters save requests.
Conflicting or unmappable semantic constraints require an explicit action to hold
them. Actual-model detection uses equivalent conversion rules and stops before any
candidate request that would lose unresolved constraints.

## Sources and validation

| Boundary | Source |
| --- | --- |
| Exact capability resolution and revision consistency | [resolve.rs](../../crates/backend/nomifun-model-invoke/src/resolve.rs) |
| Tasks, protocols and parameters | [manifest.rs](../../crates/backend/nomifun-model-invoke/src/manifest.rs) |
| Routing, adapters, URLs and authentication | [routes_table.rs](../../crates/backend/nomifun-model-invoke/src/routes_table.rs), [adapters](../../crates/backend/nomifun-model-invoke/src/adapters/mod.rs), [url_algebra.rs](../../crates/backend/nomifun-model-invoke/src/url_algebra.rs), [auth.rs](../../crates/backend/nomifun-model-invoke/src/auth.rs) |
| Registration and URL contracts | [protocol_manifest.rs](../../crates/backend/nomifun-model-invoke/tests/protocol_manifest.rs), [url_join_contract.rs](../../crates/backend/nomifun-model-invoke/tests/url_join_contract.rs) |

See [Agent Session](agent-session.md) for model transitions and native checkpoint fences, and [voice architecture](voice.md) for device and work ownership.
