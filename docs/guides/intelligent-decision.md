# Intelligent Decision (IDMM)

IDMM is an opt-in supervisor for the canonical AgentSession. It does not own a
second session or runtime. It observes durable turns/messages plus live runtime
progress and sends every recovery through the same AgentSession command boundary.

Personal Agents can save an IDMM default under **Agent Workbench → Runtime
policy**. That configuration is part of the immutable AgentPreset Revision and
is copied once when a new AgentSession is created. The new-Session screen can
override the default before launch, while the conversation capsule changes only
that Session. Later Agent edits never rewrite existing Sessions. IDMM is not a
Capability Catalog entry and grants no Tool, Context, resource, or OS permission.

## Modes

**Rule guard** calls no model. It recovers retryable provider/network failures,
detects model-stage silence, and answers explicit option prompts with the first
safe option (preferring a marked recommendation). It never auto-answers
permission, credential, purchase, or destructive prompts. A silent tool/effect
is not cancelled because doing so could duplicate an external side effect.

**Rules + bypass model** runs the same rules first. Only unresolved open questions
or ambiguous choices are sent to an explicitly selected bypass model. Its
context is bounded, it receives no tools, and its output is constrained to a
safe option, a short answer, or halt. Common API keys, tokens, passwords, and
private keys are best-effort redacted before context leaves the primary Session.

## Failover and safety

Automatic replies have a separate bubble treatment and an explanation below the
bubble: **Intelligent Decision · rule guard**, **Intelligent Decision · actual
bypass model**, or **Intelligent Decision · recovery**. Human replies keep their
usual appearance. The model name is recorded when the decision is made; changing
the current configuration does not rewrite historical explanations.

The short decision basis is kept with each decision (at most 40 Unicode
characters). Rules provide fixed wording, while the bypass model returns a short
basis in the existing decision call. Expanding it makes no additional model call.
It is collapsed by default; **Settings → System → Show intelligent decision basis
by default** changes only presentation. Each message also has its own toggle and
a link to the original question. Uncalibrated confidence percentages are omitted.

Copying an automatic reply copies only its answer. Automatic replies are excluded
from human input editing and input history. Cases requiring human input or failing
to produce an automatic answer appear as notices at the relevant point in the
conversation. The settings capsule keeps runtime policy controls and no longer
lists recent interventions. Old messages without canonical decision metadata are
not retroactively classified from their text or from audit logs.

The global Model Failover queue is frozen into each new AgentSession's immutable
chat route. The Broker retries/switches routes inside a model call; IDMM wakes the
task after a whole turn still fails. Existing Sessions are never silently rebound
when the global queue changes.

Every intervention has a stable fingerprint and idempotency key, rate and retry
budgets, and a bounded audit entry. Session deletion removes its IDMM record. If
a configured bypass provider is deleted, the Session is downgraded to rule-only.

## API

- `GET /api/agent-sessions/{id}/idmm`
- `PUT /api/agent-sessions/{id}/idmm`
- `POST /api/agent-sessions/{id}/idmm/evaluate`
