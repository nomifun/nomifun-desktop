# WebUI Connection Lifecycle Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep WebUI realtime connections alive across backend heartbeats and make authenticated browser sessions honor the existing 30-day cookie contract.

**Architecture:** Handle application-level WebSocket heartbeat frames inside the existing `httpBridge` singleton before business-event dispatch. Centralize the session duration in `nomifun-common`, then derive JWT, cookie, CSRF, and advertised WebSocket-token lifetimes from that single value.

**Tech Stack:** TypeScript, Bun test, browser WebSocket API, Rust, jsonwebtoken, Cargo test.

## Global Constraints

- Preserve desktop `x-nomi-local-trust` authentication unchanged.
- Preserve existing reconnect behavior for real transport interruptions.
- Preserve `AUTH_EXPIRED_EVENT` handling for invalidated or truly expired sessions.
- Do not merge the legacy browser bridge and `httpBridge` transports in this repair.
- Use failing regression tests before production edits.

---

### Task 1: Keep the `httpBridge` WebSocket alive

**Files:**
- Modify: `ui/src/common/adapter/httpBridge.test.ts`
- Modify: `ui/src/common/adapter/httpBridge.ts`

**Interfaces:**
- Consumes: backend application heartbeat envelope `{ name: "ping", data: { timestamp: number } }`.
- Produces: same-socket heartbeat response `{ name: "pong", data: { timestamp: number } }`.

- [ ] **Step 1: Write the failing heartbeat regression**

Import `wsEmitter`, install a controllable fake `WebSocket`, subscribe to the
`ping` event, deliver a backend ping, and assert that heartbeat handling is
internal:

```ts
test('handles application heartbeat internally', () => {
  const instances: FakeWebSocket[] = [];
  class FakeWebSocket {
    static readonly CONNECTING = 0;
    static readonly OPEN = 1;
    static readonly CLOSING = 2;
    static readonly CLOSED = 3;
    readyState = FakeWebSocket.OPEN;
    readonly sent: string[] = [];
    private readonly listeners = new Map<string, Array<(event: any) => void>>();

    constructor() {
      instances.push(this);
    }

    addEventListener(type: string, listener: (event: any) => void) {
      const listeners = this.listeners.get(type) ?? [];
      listeners.push(listener);
      this.listeners.set(type, listeners);
    }

    send(data: string) {
      this.sent.push(data);
    }

    close() {
      this.readyState = FakeWebSocket.CLOSED;
    }

    dispatch(type: string, event: unknown) {
      for (const listener of this.listeners.get(type) ?? []) listener(event);
    }
  }

  installBrowserGlobals({
    location: {
      protocol: 'http:',
      host: 'localhost:25808',
      pathname: '/sessions',
      hash: '',
    } as Location,
  });
  globalThis.WebSocket = FakeWebSocket as unknown as typeof WebSocket;

  const dispatched: unknown[] = [];
  const unsubscribe = wsEmitter('ping').on((payload) => dispatched.push(payload));
  const socket = instances[0];
  socket.dispatch('message', {
    data: JSON.stringify({ name: 'ping', data: { timestamp: 123 } }),
  });

  expect(socket.sent.length).toBe(1);
  const pong = JSON.parse(socket.sent[0]);
  expect(pong.name).toBe('pong');
  expect(typeof pong.data.timestamp).toBe('number');
  expect(dispatched.length).toBe(0);

  unsubscribe();
  socket.close();
});
```

Save and restore the real `globalThis.WebSocket` in the test file's existing
global-fixture pattern.

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
bun test ui/src/common/adapter/httpBridge.test.ts
```

Expected: FAIL because `socket.sent.length` is `0` and the ping is dispatched to
the subscribed listener.

- [ ] **Step 3: Implement the minimal same-socket pong**

In the `current` socket's message listener, immediately after resolving
`eventName` and `payload`, add:

```ts
if (eventName === 'ping') {
  if (current.readyState === WebSocket.OPEN) {
    current.send(
      JSON.stringify({
        name: 'pong',
        data: { timestamp: Date.now() },
      })
    );
  }
  return;
}
```

- [ ] **Step 4: Run the test and verify GREEN**

Run:

```bash
bun test ui/src/common/adapter/httpBridge.test.ts
```

Expected: all `httpBridge` tests PASS.

- [ ] **Step 5: Commit the independently working heartbeat repair**

```bash
git add ui/src/common/adapter/httpBridge.ts ui/src/common/adapter/httpBridge.test.ts
git commit -m "fix(ui): keep realtime WebSocket heartbeat alive"
```

### Task 2: Unify authenticated session lifetime

**Files:**
- Modify: `crates/backend/nomifun-common/src/constants.rs`
- Modify: `crates/backend/nomifun-auth/src/jwt.rs`
- Modify: `crates/backend/nomifun-auth/src/cookie.rs`
- Modify: `crates/backend/nomifun-auth/src/routes.rs`
- Modify: `docs/reference/configuration.md`
- Modify: `docs/reference/configuration.zh.md`

**Interfaces:**
- Produces: `SESSION_MAX_AGE_SECONDS: u64`, derived from `COOKIE_MAX_AGE_DAYS`.
- Consumes: JWT signing, session-cookie creation, CSRF-cookie creation, and WebSocket-token lifetime reporting.

- [ ] **Step 1: Write the failing JWT lifetime regression**

Extend `sign_and_verify_roundtrip` in `jwt.rs`:

```rust
assert_eq!(
    payload.exp - payload.iat,
    u64::from(COOKIE_MAX_AGE_DAYS) * 24 * 60 * 60,
    "JWT lifetime must match the browser session cookie contract",
);
```

Import `COOKIE_MAX_AGE_DAYS` inside the `#[cfg(test)]` module from
`nomifun_common::constants`.

- [ ] **Step 2: Run the test and verify RED**

Run:

```bash
cargo test -p nomifun-auth jwt::tests::sign_and_verify_roundtrip --lib
```

Expected: FAIL with left value `86400` and right value `2592000`.

- [ ] **Step 3: Centralize and consume the session duration**

In `nomifun-common/src/constants.rs`, replace the stale session descriptor and
add a derived seconds constant:

```rust
pub const SESSION_EXPIRY: &str = "30d";
pub const COOKIE_NAME: &str = "nomifun-session";
pub const COOKIE_MAX_AGE_DAYS: u32 = 30;
pub const SESSION_MAX_AGE_SECONDS: u64 = COOKIE_MAX_AGE_DAYS as u64 * 24 * 60 * 60;
```

In `jwt.rs`, replace the 24-hour literal:

```rust
use nomifun_common::constants::SESSION_MAX_AGE_SECONDS;

/// JWT token lifetime, kept identical to the browser session cookie.
const TOKEN_EXPIRY: Duration = Duration::from_secs(SESSION_MAX_AGE_SECONDS);
```

In `cookie.rs`, use `SESSION_MAX_AGE_SECONDS` for both cookie builders. In
`routes.rs`, report `SESSION_MAX_AGE_SECONDS * 1000` for the WebSocket-token
lifetime. Update both configuration references from `24h` to `30d` and state
that JWT and cookie lifetimes are identical.

- [ ] **Step 4: Run backend tests and verify GREEN**

Run:

```bash
cargo test -p nomifun-auth --lib
cargo test -p nomifun-realtime --lib
```

Expected: all tests PASS.

- [ ] **Step 5: Commit the independently working session repair**

```bash
git add crates/backend/nomifun-common/src/constants.rs \
  crates/backend/nomifun-auth/src/jwt.rs \
  crates/backend/nomifun-auth/src/cookie.rs \
  crates/backend/nomifun-auth/src/routes.rs \
  docs/reference/configuration.md \
  docs/reference/configuration.zh.md
git commit -m "fix(auth): align WebUI session lifetimes"
```

### Task 3: Verify the complete lifecycle repair

**Files:**
- Verify only: all files changed by Tasks 1 and 2.

**Interfaces:**
- Consumes: stable heartbeat behavior and unified 30-day session contract.
- Produces: a clean, type-safe, tested working tree ready for delivery.

- [ ] **Step 1: Run all targeted regressions**

```bash
bun test ui/src/common/adapter/httpBridge.test.ts
cargo test -p nomifun-auth --lib
cargo test -p nomifun-realtime --lib
```

Expected: all tests PASS.

- [ ] **Step 2: Run UI type checking**

```bash
bun run --filter=./ui typecheck
```

Expected: exit code `0` with no TypeScript errors.

- [ ] **Step 3: Validate formatting and repository state**

```bash
cargo fmt --all -- --check
git diff --check
git status --short --branch
```

Expected: formatting and diff checks exit `0`; status contains only the planned
commits and no uncommitted production or test changes.
