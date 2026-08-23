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
supports. A successful health request does not prove that every media, tool, or
streaming operation is compatible.

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
| Embedding / reranking | Retrieval and knowledge workflows |

Task selection is explicit. The runtime does not infer image or video support
from a model name, and it does not silently use a same-named model from another
provider.

Creative Studio persists the exact `{ providerId, model, task, capability }`
identity with each admitted media operation. Retrying the same idempotent task
cannot change those facts.

## NomiFun Free Models

When available in the current build, **NomiFun Free Models** use a built-in
managed provider. You can enable the service, refresh its model catalog, run a
health check, and activate an available model without first creating a custom
provider entry or supplying your own API key.

These remain online third-party inference services. Availability, quota,
latency, and data-handling terms can change. Read the in-product notice before
sending sensitive content.

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
