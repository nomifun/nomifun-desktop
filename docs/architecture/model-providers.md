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
reviewed provider by provider on 2026-10-15. Prefer `openai.responses` where the
service confirms and recommends it; services primarily or only documenting Chat
Completions retain `openai.chat_text`. Metered plans, subscriptions, regions and
exact models remain distinct. Custom/New API discovery tries Responses first;
the NomiFun gateway retains its catalog's exact protocol declarations. Saved protocols are never rewritten automatically.
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

## Actual protocol verification during setup

An explicit action calls `/api/providers/detect-chat-protocol` or the saved
provider's corresponding subroute with the exact model and capability draft.
It reuses production OpenAI/Responses serializers and stream decoders, checking
the current protocol before trying the alternative on confirmed incompatibility.
Reachability, empty-JSON probes, authentication, quota and transient failures
cannot create a verified choice. The returned capability is the URL, endpoint
and parameters actually tested. Model limits remain unchanged; the response
separately declares the bounded verification output budget.

This action creates no Session, writes no catalog and grants no runtime protocol
fallback. Saved providers use server-side encrypted credentials and revision
fences; new providers use the explicitly supplied connections. The editor checks
the draft snapshot before applying results, blocks duplicate submissions and
discards stale results after model, credential or manual changes. A click and a
usage notice precede real requests; normal typing and reachability discovery do
not trigger billable model calls.
Cross-protocol candidates must retain output and thinking semantics. When native
parameters cannot be transferred safely, selection stops without changing the draft.

## Sources and validation

| Boundary | Source |
| --- | --- |
| Exact capability resolution and revision consistency | [resolve.rs](../../crates/backend/nomifun-model-invoke/src/resolve.rs) |
| Tasks, protocols and parameters | [manifest.rs](../../crates/backend/nomifun-model-invoke/src/manifest.rs) |
| Routing, adapters, URLs and authentication | [routes_table.rs](../../crates/backend/nomifun-model-invoke/src/routes_table.rs), [adapters](../../crates/backend/nomifun-model-invoke/src/adapters/mod.rs), [url_algebra.rs](../../crates/backend/nomifun-model-invoke/src/url_algebra.rs), [auth.rs](../../crates/backend/nomifun-model-invoke/src/auth.rs) |
| Registration and URL contracts | [protocol_manifest.rs](../../crates/backend/nomifun-model-invoke/tests/protocol_manifest.rs), [url_join_contract.rs](../../crates/backend/nomifun-model-invoke/tests/url_join_contract.rs) |

See [Agent Session](agent-session.md) for model transitions and native checkpoint fences, and [voice architecture](voice.md) for device and work ownership.
