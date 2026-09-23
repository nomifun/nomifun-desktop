# Model Management, Routing, and Failover

NomiFun's **Models** surface is an extensible control plane, not a fixed vendor
list. It separates provider credentials, model records, task capabilities, and
reliability policy so the same catalog can be reused by conversations,
companions, scheduled work, presets, and Creative Studio.

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
such as Ollama or vLLM. Register only capabilities the endpoint actually
supports. A successful health request does not prove that every media or
technical operation is compatible.

For each model:

1. Choose or create the provider.
2. Enter the endpoint and credentials required by that provider.
3. Add the exact model id.
4. Override context or output limits when the upstream default is missing or
   inaccurate.
5. Enable only the tasks that the provider/protocol contract supports.
6. Save and run the available health/status checks.

Provider credentials remain local configuration. Any hosted provider still
processes the content sent to it according to its own billing and data policy.

## Task-aware capabilities

The managed model catalog can represent these task families:

| Task family | Typical consumers |
| --- | --- |
| Chat / agent turns | Conversations, companions, presets, scheduled work, Canvas Assistant |
| Realtime | Low-latency interactive surfaces supported by the provider |
| Vision | Image-aware chat and analysis |
| Speech recognition (ASR) | Voice input and companion/device speech |
| Speech synthesis (TTS) | Companions, devices, and Canvas audio nodes |
| Image generation / editing | Creative Studio Canvas and Image Workbench |
| Video generation | Creative Studio Canvas and Video Workbench |
| Music generation | Conversation creation and Creative Studio |
| Embedding / reranking | Retrieval and knowledge workflows |

Task selection is explicit. The runtime does not infer image or video support
from a model name, and it does not silently use a same-named model from another
provider.

The only user-authored refinements on a Chat capability are image understanding,
video understanding, audio input, and provider-native web search. Tool calling,
reasoning, and streaming are not checkboxes. Chat models start optimistic for
those technical capabilities. The runtime records a negative observation only
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
| Image understanding | `models.default.vision` | One Chat capability declaring `vision_input`, with no confirmed tool-calling limitation |
| Image generation | `models.default.imageGeneration` | `image_generation` |
| Image editing | `models.default.imageEdit` | `image_edit` |
| Video generation | `models.default.videoGeneration` | `video_generation` |
| Music generation | `models.default.musicGeneration` | `music_generation` |
| Speech synthesis | `models.default.speechSynthesis` | `speech_synthesis` |

Automatic media Actions use the exact task default when one is configured. With
no default, they select an enabled model that declares the required task,
preferring a healthy capability observation, then the provider and model order
in Model Management. If no compatible model is available, the conversation asks
the user to configure one. An unavailable configured default is not silently
replaced. Professional creation surfaces may still select another compatible
model for one explicit task.

The vision model is frozen into new conversations as a conditional Chat
candidate. It participates only when the current request actually requires
image input, so it cannot become an ordinary text-chat failover. A primary Chat
model that already supports vision remains the direct route.

The model capability catalog and routing authority are separate layers.
Creation tasks and multimodal inputs require positive evidence before automatic
routing. Tool calling, reasoning, and streaming are optimistic until conclusive
negative evidence narrows later routes. That optimism can never promote a Chat
model into image/video/music/TTS/ASR generation: automatic creation selects
only models that explicitly support the exact task.

Creative Studio persists the exact `{ providerId, model, task, capability }`
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
- IDMM policy:
  `crates/backend/nomifun-idmm/src/policy.rs`
- Creative Studio model catalog:
  `ui/src/renderer/pages/creativeStudio/models/catalog.ts`
