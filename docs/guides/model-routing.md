# Model Failover Queue

The current feature behind model-routing settings is a **model failover queue**,
not a credential round-robin pool.

It lets Nomi-engine conversations try a configured sequence of backup models
when a provider fault is detected. ACP/CLI agents are not included in this
feature because their provider calls happen inside external runtimes.

## What It Does

- Stores a global default queue under `agent.model_failover`.
- Allows per-conversation overrides under `extra.model_failover`.
- Applies only to Nomi-engine conversations.
- Can be used by IDMM fault-watch flows when that session has failover enabled.
- Does not distribute load across API keys.
- Does not make all CLI agents share a common pool.

## When To Use It

Use model failover when a Nomi-engine session should recover from transient
provider/model faults without requiring manual model switching.

Typical queue:

```text
primary model -> cheaper backup -> stronger backup -> manual review
```

The queue is about reliability, not quota aggregation. If every configured
provider is down or the prompt/tool state is invalid, failover cannot make the
turn succeed.

## How It Relates To IDMM

IDMM has separate fault and decision watches. Model failover belongs to the
fault side: when a provider fault is classified as recoverable and failover is
enabled, IDMM can ask the conversation runtime to retry through the configured
queue.

AutoWork then sits one layer above both features: it keeps a tagged work queue
moving, while IDMM/model failover try to keep each claimed turn alive.

## Source Of Truth

- `crates/backend/nomifun-conversation/src/model_failover.rs`
- `crates/backend/nomifun-conversation/src/failover_seam.rs`
- `crates/backend/nomifun-app/src/router/model_failover.rs`
- `crates/backend/nomifun-idmm/src/policy.rs`

Older copies of this page described multi-credential round-robin routing. That
was not the current implementation and should not be used as operator guidance.
