# Intelligent Decision (IDMM)

**IDMM** — Intelligent Decision-Making Mode — is Nomi's reliability layer for
unattended work. It is a **session supervisor** that watches each turn and
intervenes the moment it stalls, so a long, automated run reaches a terminal
state instead of hanging on a provider hiccup or a model that has stopped
making progress.

If [AutoWork](autowork-requirements.md) is the engine that drives work
*forward*, IDMM is the guard that keeps each turn *moving*. The two are designed
to compose: AutoWork claims and executes requirements; IDMM makes sure every
turn it starts actually finishes.

> IDMM is an **optional** supervisor (the `nomifun-idmm` crate). You turn it on
> per session, from the same place you toggle AutoWork — the session header.

## Why it exists

Agent turns fail in boring, recoverable ways far more often than they fail in
interesting ones:

- a provider returns a transient `429` / `5xx` and the turn would otherwise give
  up;
- the model retries the same failing call in a loop;
- the model spins on a tool call and never decides what to do next;
- the turn simply goes quiet and would eventually hit a hard timeout.

For an interactive session you would just nudge it yourself. For an *unattended*
session — an AutoWork queue running overnight, a scheduled job, a multi-agent
teammate — there is nobody watching. IDMM is that watcher.

## The two tiers

When IDMM detects a stall it resolves it with the cheapest mechanism that can,
escalating only when it must.

### Rule tier (no LLM)

A deterministic policy handles the common, mechanical stalls **without calling a
model at all** — so it is fast and free:

- **Provider faults** — transient errors and rate limits are absorbed and the
  turn is retried under a sane backoff instead of failing outright.
- **Retry loops** — repeated identical retries are detected and broken.
- **Tool-spin** — a model that keeps re-issuing the same tool call without
  progress is steered back on track.

Most interventions never get past this tier.

### Sidecar tier (a backup model)

When a stall is genuinely a *decision* problem — the main model is stuck and a
rule cannot resolve it — IDMM asks a **lightweight sidecar model** to make the
next decision so the session does not deadlock. The sidecar is a small, cheap
"second opinion" model: its only job is to unstick the turn, not to take over
the work.

This is the **bypass model** in product terms: a model that sits beside the main
agent and steps in only when needed.

## Session guard & keep-alive

Together, the rule tier and the sidecar form the **session guard**: IDMM keeps
the target alive through faults and decision stalls and shepherds the turn to a
terminal state. This is what "session keep-alive" means in Nomi — not a dumb
heartbeat, but an active supervisor that resolves the thing that would otherwise
have stalled the turn.

## How it composes with AutoWork

IDMM and AutoWork are independent but complementary:

- **AutoWork** claims the next requirement, injects it, waits for the turn to
  finish, and finalises it.
- When AutoWork starts a turn, it asks IDMM (if wired) to **ensure supervision**
  of that target for the duration of the turn.
- **IDMM** keeps that turn from getting stuck, so it reaches `done` / `failed`
  cleanly instead of timing out.

The net effect: AutoWork provides forward progress; IDMM provides liveness. A
queue can run for hours, unattended, and individual transient failures no longer
abort the run.

```
AutoWork: claim ─▶ inject ─▶ [ turn runs ] ─▶ finalize (done/failed)
                                  ▲
                                  │ ensure supervision
                              IDMM guard ──▶ rule tier ──▶ (escalate) ──▶ sidecar model
```

See `crates/backend/nomifun-idmm/` for the per-tier policy detail and the
intervention log API.

## Enabling it

IDMM is toggled from the **session header**, the same control surface as
AutoWork. Turn it on for a conversation or a terminal target that you intend to
leave running unattended. There is nothing to configure for the rule tier; the
sidecar tier uses a lightweight model from your configured providers.

## When to use it

- **Use it** for any unattended run: AutoWork queues, scheduled
  ([cron](scheduled-tasks.md)) jobs, overnight batches, or long terminal-driven
  agent sessions.
- **You may not need it** for short, interactive sessions where you are watching
  the turn and can intervene yourself.

## See also

- [AutoWork & Requirements](autowork-requirements.md) — the engine IDMM most
  often guards.
- [Scheduled Tasks](scheduled-tasks.md) — unattended jobs that benefit from
  supervision.
- [Terminal](terminal.md) — IDMM can supervise long-running terminal targets.
