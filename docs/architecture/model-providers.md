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

Tasks under one provider may use different domains, named connections and credentials. `base_url_override` affects only the selected task; `endpoint`, `poll_endpoint`, `content_endpoint` and `realtime_endpoint` represent submission, polling, content and realtime transport. Absolute overrides with a different credential origin require explicit `allow_cross_origin_credentials`. Changing protocols clears old transport overrides, cross-origin consent and protocol parameters before applying the new manifest.

`provider_params` obeys the protocol's strict encoding contract and cannot carry credentials or local transport fields. Values that cannot be encoded losslessly are rejected on save. Responses expose `has_credentials` without returning secrets. Invocation-graph changes atomically increment `config_revision`; resolution, asynchronous jobs and health observations use a consistent revision, so stale probes cannot overwrite new configuration.

“OpenAI compatible” does not prove compatibility across modalities. Ark versus Volcengine voice, native DashScope and MiniMax regional connections are expressed by their adapters and manifests, without a duplicate endpoint matrix. Provider changes update the registry, serializer and focused contract tests, then require actual-account verification. Local substitutes do not prove cloud permissions or model experience.

## Sources and validation

| Boundary | Source |
| --- | --- |
| Exact capability resolution and revision consistency | [resolve.rs](../../crates/backend/nomifun-model-invoke/src/resolve.rs) |
| Tasks, protocols and parameters | [manifest.rs](../../crates/backend/nomifun-model-invoke/src/manifest.rs) |
| Routing, adapters, URLs and authentication | [routes_table.rs](../../crates/backend/nomifun-model-invoke/src/routes_table.rs), [adapters](../../crates/backend/nomifun-model-invoke/src/adapters/mod.rs), [url_algebra.rs](../../crates/backend/nomifun-model-invoke/src/url_algebra.rs), [auth.rs](../../crates/backend/nomifun-model-invoke/src/auth.rs) |
| Registration and URL contracts | [protocol_manifest.rs](../../crates/backend/nomifun-model-invoke/tests/protocol_manifest.rs), [url_join_contract.rs](../../crates/backend/nomifun-model-invoke/tests/url_join_contract.rs) |

See [Agent Session](agent-session.md) for model transitions and native checkpoint fences, and [voice architecture](voice.md) for device and work ownership.
