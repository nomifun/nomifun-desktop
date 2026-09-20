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
