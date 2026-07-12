# WebUI Connection Lifecycle Repair Design

## Problem

WebUI has two independent time-based connection failures.

First, `ui/src/common/adapter/httpBridge.ts` owns the WebSocket used by the
shared `ipcBridge` emitters. The backend sends an application-level `ping`
every 30 seconds and closes clients that have not replied with `pong` for more
than 60 seconds. The legacy browser bridge replies, but the `httpBridge`
connection does not. Consequently, the primary realtime connection is closed
and reconnected about once per minute. Events emitted during each gap are not
replayed, so conversations, terminals, and other realtime UI state can appear
stalled or failed.

Second, the browser session cookie advertises a 30-day lifetime while its JWT
expires after 24 hours. The WebUI has no usable cookie renewal mechanism: the
token is stored in an HttpOnly cookie, while the legacy refresh endpoint expects
the token in a JSON body. After 24 hours, the UI can still look authenticated,
but protected HTTP requests return `403 Invalid or expired token` and the
WebSocket is rejected.

## Goals

- Keep the `httpBridge` WebSocket healthy for as long as the browser and server
  remain reachable.
- Preserve the existing reconnect behavior for real transport interruptions.
- Make the signed JWT lifetime match the existing 30-day session-cookie
  contract.
- Preserve the existing shared authentication-expiry event, state reset, and
  login redirect for genuinely expired or invalid sessions.
- Add regressions that fail on both current defects.

## Non-goals

- Do not merge the legacy browser bridge and `httpBridge` WebSocket transports
  in this repair. That is a larger migration with more consumers and does not
  need to block the lifecycle fix.
- Do not introduce access-token and refresh-token persistence, rotation, or a
  new database schema.
- Do not change desktop local-trust authentication.
- Do not add replay semantics to the realtime backend.

## Design

### WebSocket heartbeat

The `httpBridge` message handler will recognize the backend's application-level
`ping` event before normal event dispatch. When the socket instance that
received the message is still open, it will immediately send:

```json
{"name":"pong","data":{"timestamp":0}}
```

The timestamp value will be generated at response time with `Date.now()`. The
reply will use the captured `current` socket rather than the mutable module-level
`ws` reference. This prevents a late message from an old socket from writing to
a replacement connection.

The heartbeat message will not be forwarded to business listeners. All other
events continue through the existing `wsListeners` dispatch path unchanged.
Close and error handling retain the current exponential reconnect behavior.

### Session lifetime

`nomifun-auth` will derive `TOKEN_EXPIRY` from
`nomifun_common::constants::COOKIE_MAX_AGE_DAYS` instead of maintaining a
separate 24-hour literal. With the current product constant, JWTs, session
cookies, CSRF cookies, and the WebSocket token lifetime all represent 30 days.

Password changes continue to rotate the signing secret and invalidate every
existing session immediately. Explicit logout continues to blacklist the
presented token. Already-expired 24-hour tokens cannot be retroactively
extended; affected users must log in once after receiving the repaired build.

## Data flow

1. The backend heartbeat manager sends `{"name":"ping", ...}`.
2. `httpBridge` parses the event and sends `{"name":"pong", ...}` on the same
   open WebSocket.
3. The backend receive loop records the pong and updates that connection's
   heartbeat timestamp.
4. The next timeout check sees a fresh timestamp and keeps the connection.

For authentication:

1. Login, first-run setup, or QR login signs a JWT.
2. JWT `exp - iat` is calculated from the same 30-day constant used by the
   session and CSRF cookies.
3. HTTP middleware and the realtime token validator accept the token through
   the same deadline.
4. Invalidated or truly expired sessions continue into the existing
   `AUTH_EXPIRED_EVENT` logout flow.

## Error handling

- A pong send is attempted only while the captured socket is `OPEN`. If the
  connection closes between receipt and send, the normal close handler owns
  recovery.
- Malformed WebSocket payloads remain ignored, matching current behavior.
- Close code `1008` is not treated as authentication failure by code alone in
  `httpBridge`, because the backend also uses it for heartbeat timeouts. The
  explicit `auth-expired` event and HTTP 403 path remain authoritative.
- Desktop requests retain `x-nomi-local-trust` behavior and are not redirected
  into WebUI login handling.

## Testing

### Frontend regression

Extend `ui/src/common/adapter/httpBridge.test.ts` with a fake WebSocket that:

- opens the `wsEmitter` connection;
- delivers an application-level `ping` message;
- asserts that the same socket sends exactly one `pong` envelope;
- asserts that the heartbeat is not delivered to a business listener.

The test must fail before implementation because the current handler sends no
reply.

### Backend regression

Extend `crates/backend/nomifun-auth/src/jwt.rs` tests to assert that a signed
token's `exp - iat` equals
`COOKIE_MAX_AGE_DAYS * 24 * 60 * 60` seconds.

The test must fail before implementation because the current token lifetime is
24 hours.

### Verification

Run:

- `bun test ui/src/common/adapter/httpBridge.test.ts`
- `cargo test -p nomifun-auth --lib`
- `cargo test -p nomifun-realtime --lib`
- `bun run --filter=./ui typecheck`
- `git diff --check`

## Acceptance criteria

- The `httpBridge` connection replies to every backend application-level ping
  without exposing ping as a business event.
- A healthy WebUI realtime connection is no longer closed at the 60-second
  heartbeat timeout.
- Newly signed WebUI JWTs remain valid for exactly the configured cookie
  lifetime, currently 30 days.
- Existing HTTP and WebSocket authentication-expiry UX remains intact.
- Targeted tests, backend tests, UI type checking, and diff validation pass.
