# Model Management, Routing, and Failover

NomiFun's **Models** surface is an extensible control plane, not a fixed vendor
list. It separates provider credentials, model records, task capabilities, and
reliability policy so the same catalog can be reused by conversations,
companions, scheduled work, presets, and Creation.

> Simplified Chinese: [model-routing.zh.md](model-routing.zh.md)

## What the catalog manages

Open **Models** (`/models`) to manage:

- provider endpoint, protocol, authentication, and provider-level parameters;
- model id, enabled state, context window, output limit, and task capabilities;
- local speech-recognition models where supported by the current build;
- global defaults, IDMM settings, and the ordered model failover queue.

Execution engines are a separate concern. Nomi, Claude Code, Codex, OpenCode,
OpenClaw, and other execution backends answer "who performs the work"; Model
Management answers "which model and capability does that work use".

## Connect cloud, compatible, and self-hosted services

The native backend set includes Anthropic, OpenAI-compatible, Amazon Bedrock,
and Google Vertex. The provider catalog also supplies presets and protocol
profiles for many services.

An OpenAI-compatible or otherwise registered protocol can use a custom base URL
to reach a cloud gateway, a private endpoint, or a local/self-hosted service
such as Ollama or vLLM. Distinct tasks can have their own invocation routes. A
successful health request does not prove that every media or technical operation
is compatible.

For each model:

1. Choose or create the provider.
2. Enter the endpoint and credentials required by that provider.
3. Search the provider's complete model catalog or enter the exact model id.
4. Confirm the invocation purpose. Specialized ASR, TTS, and other entry points
   carry their purpose; an unknown model in the general entry asks once after
   the model id is entered.
5. For Chat models, set the context window and maximum output side by side in **Context & output**
   on the model configuration homepage, with the compaction threshold below.
   Unset overrides keep the provider/model defaults.
6. Save and run the available health/status checks.

Provider credentials remain local configuration. Any hosted provider still
processes the content sent to it according to its own billing and data policy.

Chat, speech recognition, speech synthesis, and the other specialized model
pages provide **Edit model** on each model row. Edit and save without leaving
the current page. The editor shows the current task's invocation route and
preserves the latest configuration for other tasks. Model aliases and
descriptions are shared across tasks. Use **Providers & keys** to manage
credentials or the model's complete set of invocation routes.

## Connect an optional model gateway

The **NomiFun Model Gateway** provider preset connects to gateway software you
self-host or a service independently operated by a community member. The
official NomiFun project provides the open-source software and protocol, does
not conduct commercial operations, never charges for project features, and
does not operate paid services. Any model-token costs are paid directly to the
model service you choose. Existing providers and manual configuration remain
available.

Choose the gateway preset, enter its **runtime URL**, and check the operator
using the anonymous `/nomifun/v1/meta` endpoint. Then enter your **API key** and
read the authenticated `/nomifun/v1/catalog`. The flow lets you select the models to import,
and saves the provider, selected models, invocation routes, and connections
together. Catalog metadata describes the service's declared capabilities;
importing it does not prove successful model invocation.

Each imported model keeps its native protocol. The catalog protocol mapping is:

| Gateway protocol | Desktop invocation protocol | Connection |
| --- | --- | --- |
| `openai` | `openai.chat_text` | `default` |
| `openai-response` | `openai.responses` | `default` |
| `anthropic` | `anthropic.messages` | `anthropic` |
| `gemini` | `gemini.generate_text` | `gemini` |

The provider stores three connections, all using the key you supplied:

- `default`: Bearer authentication, with `/v1` appended to the runtime base URL;
- `anthropic`: `x-api-key` header authentication at the runtime URL root;
- `gemini`: `x-goog-api-key` header authentication at the runtime URL root.

Other supported standard tasks use the `default` connection. The gateway's
metadata, catalog, and account APIs are control endpoints, not extra invocation
connections. Account information is fetched from `/nomifun/v1/account` on demand
with the configured gateway key.

Rotate the gateway key through the provider settings so all three connections
are updated in one save. **Sync model catalog** adds new models and updates
catalog metadata you have not edited. It preserves your edits and never deletes
models. Neither action
certifies model health; run the available invocation checks for the routes
you intend to use.

Gateway metadata can provide website, key-management, recharge, and billing
links. Only HTTPS links open, and they open in your system browser. Desktop
does not embed a provider login, accept a key through a deep link, or require
a gateway account to use other providers. You enter or rotate your key in
Desktop yourself.

## Model catalog and invocation routes

The catalog supplies suggestions. It neither restricts which model ids can be
saved nor requires a "Supported tasks" selection first. Missing suggestions or
a failed catalog request do not block manual entry. Specialized ASR, TTS, and
other entry points carry the invocation purpose. A model with no verified task
in the general entry asks for that purpose once after the id is entered; it is
never automatically treated as a Chat model. Only provider-declared task data
or an exact documented profile can automatically suggest distinct routes.
Future model ids from a native catalog need no built-in name whitelist.
Name-based inference never creates
routes automatically. Use "Add invocation route" to configure tasks that need
a different protocol, endpoint, or credentials.

Manually changing the model id clears automatically suggested routes for the
old id while retaining user-configured and previously saved routes. A verified
catalog purpose that conflicts with the current purpose requires explicit
acknowledgment; the current purpose remains until then. Background catalog
refreshes never replace the existing configuration.

The managed model catalog can represent these task families:

| Task family | Typical consumers |
| --- | --- |
| Chat / agent turns | Conversations, companions, presets, scheduled work, Canvas Assistant |
| Realtime | Low-latency interactive surfaces supported by the provider |
| Vision | Image-aware chat and analysis |
| Speech recognition (ASR) | Voice input and companion/device speech |
| Speech synthesis (TTS) | Companions, devices, and Canvas audio nodes |
| Image generation / editing | Creation Canvas nodes |
| Video generation | Creation Canvas nodes |
| Music generation | Conversation creation and Creation |
| Embedding / reranking | Retrieval and knowledge workflows |

These tasks identify distinct invocation protocols and endpoints. The runtime
does not infer image or video generation routes from a model name, and it does
not silently use a same-named model from another provider.

The backend checks whether the selected protocol supports the purpose when
saving. Invocations read only the saved route for the exact task. Missing ASR,
TTS, or other independent routes produce an explicit error; they never fall
back to a Chat HTTP endpoint.

Chat requires no image, video, audio, tool calling, reasoning, streaming, or
provider-native web search checkboxes. Catalog capability metadata is advisory;
an omitted trait in an existing configuration does not disable a model feature.
Input content and request options depend on the formats the registered protocol
can serialize and on the actual provider response. A missing serializer is not
made available by removing a checkbox. Provider-native search likewise requires
an implementation for that protocol.

Chat models start optimistic for tool calling, reasoning, and streaming. The
runtime records a negative observation only
when a complete provider error object returns HTTP 400/422 and its
machine-readable fields explicitly identify the unsupported parameter or
feature. Authentication/permission failures, rate limits, quota, timeouts,
network faults, 5xx responses, and natural-language diagnostic text never
downgrade a capability.

Negative observations are written both to the capability's durable health JSON
and to the process-local route cache. Later routes remove confirmed unsupported
tool/reasoning features; a confirmed streaming limitation uses a bounded single
JSON response. Changing invocation or connection configuration clears the old
observation so the new configuration starts optimistic again. Realtime remains
the independent `realtime_conversation` task and protocol, never a Chat trait.

## Capability routing inside a conversation

The General Agent is preset with image generation, image editing, video
generation, speech synthesis, and music generation Actions. An explicit
creation request in an ordinary conversation prefers the user's exact default
for that task:

| Conversation need | Default key | Required capability |
| --- | --- | --- |
| Image understanding | `models.default.vision` | A Chat protocol that can represent image input, with no confirmed tool-calling limitation |
| Image generation | `models.default.imageGeneration` | `image_generation` |
| Image editing | `models.default.imageEdit` | `image_edit` |
| Video generation | `models.default.videoGeneration` | `video_generation` |
| Music generation | `models.default.musicGeneration` | `music_generation` |
| Speech synthesis | `models.default.speechSynthesis` | `speech_synthesis` |

Automatic media Actions use the exact task default when one is configured. With
no default, they select an enabled model with the required invocation route,
preferring a healthy capability observation, then the provider and model order
in Model Management. If no compatible model is available, the conversation asks
the user to configure one. An unavailable configured default is not silently
replaced. Professional creation surfaces may still select another compatible
model for one explicit task.

The vision model is frozen into new conversations as a conditional Chat
candidate. It participates only when the current request actually requires
image input, so it cannot become an ordinary text-chat failover. A primary Chat
model whose Chat protocol can represent image input remains the preferred route.
Missing catalog metadata or old traits do not force another model; actual
support is still determined by the provider response.

The model catalog and invocation configuration are separate layers. Multimodal
Chat input depends on protocol formats and actual invocation results; creation
tasks still require their own saved routes. Tool calling, reasoning, and
streaming are optimistic until conclusive negative evidence narrows later
routes. Chat input and technical features do not create image/video/music/TTS/ASR
generation routes: automatic creation selects only models with the exact task
route configured.

Creation persists the exact `{ providerId, model, task, capability }`
identity with each admitted media operation. Retrying the same idempotent task
cannot change those facts.

## Model Failover Queue

The failover feature is an ordered reliability queue, not a credential
round-robin pool.

It:

- stores a global default queue under `agent.model_failover`;
- allows per-conversation overrides under `extra.model_failover`;
- can be used by IDMM fault-watch when that session has failover enabled;
- does not distribute load across API keys.

A typical queue is:

```text
primary model -> inexpensive backup -> stronger backup -> manual review
```

The current runtime permits up to four switches across the queue. If every
configured provider is down, the required task is unsupported, or the
prompt/tool state is invalid, failover cannot make the turn succeed.

## How it relates to IDMM and AutoWork

IDMM has separate fault and decision watches. Model failover belongs to the
fault side: when a provider fault is classified as recoverable and failover is
enabled, IDMM can ask the conversation runtime to retry through the configured
queue.

AutoWork sits one layer above both features. It keeps a tagged requirement queue
moving, while IDMM and model failover try to keep each claimed turn alive.

External ACP/CLI agents do not participate in the Nomi engine failover queue;
their provider calls happen inside their own runtime.

## Source of truth

- Provider and model settings UI:
  `ui/src/renderer/pages/modelHub/`
- Shared model storage types:
  `ui/src/common/config/storage.ts`
- Model failover:
  `crates/backend/nomifun-conversation/src/model_failover.rs`
- Failover API:
  `crates/backend/nomifun-app/src/router/model_failover.rs`
- IDMM supervision service:
  `crates/backend/nomifun-idmm/src/service.rs`
- Creation model catalog:
  `ui/src/renderer/pages/creativeStudio/models/catalog.ts`
