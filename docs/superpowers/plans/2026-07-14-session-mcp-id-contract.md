# Session MCP Identifier Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make selected MCP server snapshots use string identifiers end-to-end, while accepting legacy numeric identifiers only at the backend compatibility boundary.

**Architecture:** The frontend catalog entity remains database-shaped (`id: number`), but its projection to a session snapshot explicitly stringifies the ID. The backend keeps the canonical `SessionMcpServer.id: String` type and uses a scoped custom deserializer to normalize legacy integer JSON values. Conversation status IDs use the same string contract in the frontend.

**Tech Stack:** TypeScript, Bun test runner, Rust, serde, Tokio tests.

## Global Constraints

- Keep `IMcpServer.id` numeric because it is the catalog database primary key.
- Keep `SessionMcpServer.id` string because session identifiers are polymorphic.
- Accept only JSON strings and integers for the legacy compatibility path; reject floats, booleans, arrays, objects, and null.
- Do not change MCP transport validation or selection semantics.

---

### Task 1: Normalize the frontend session snapshot contract

**Files:**
- Modify: `ui/src/common/config/storage.ts:671-679`
- Modify: `ui/src/renderer/hooks/mcp/catalog.ts:55-58`
- Create: `ui/src/renderer/hooks/mcp/catalog.test.ts`

**Interfaces:**
- Consumes: `IMcpServer.id: number`.
- Produces: `ISessionMcpServer.id: string` and `toSessionMcpServer(server): ISessionMcpServer`.

- [ ] **Step 1: Write the failing test**

```ts
test('serializes a catalog integer id as a session string id', () => {
  expect(toSessionMcpServer({ id: 3, name: 'search', transport })).toMatchObject({ id: '3', name: 'search', transport });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bun test ui/src/renderer/hooks/mcp/catalog.test.ts`

Expected: failure because the current result has `id: 3`.

- [ ] **Step 3: Write minimal implementation**

```ts
export interface ISessionMcpServer {
  id: string;
  name: string;
  transport: IMcpServerTransport;
}

export const toSessionMcpServer = (server: Pick<IMcpServer, 'id' | 'name' | 'transport'>): ISessionMcpServer => ({
  id: String(server.id),
  name: server.name,
  transport: server.transport,
});
```

Also change `IConversationMcpStatus.id` to `string`.

- [ ] **Step 4: Run test to verify it passes**

Run: `bun test ui/src/renderer/hooks/mcp/catalog.test.ts`

Expected: PASS.

### Task 2: Add backend compatibility at the session snapshot boundary

**Files:**
- Modify: `crates/backend/nomifun-api-types/src/agent_build_extra.rs:38-42`
- Modify: `crates/backend/nomifun-conversation/src/service_test.rs` in the create-test section

**Interfaces:**
- Consumes: JSON `selected_session_mcp_servers[].id` as string (current) or integer (legacy).
- Produces: `SessionMcpServer.id: String`; persisted `extra.session_mcp_servers[].id` is always a JSON string.

- [ ] **Step 1: Write failing tests**

```rust
#[tokio::test]
async fn create_normalizes_legacy_numeric_session_mcp_ids_to_strings() {
    let req = serde_json::from_value(json!({
        "type": "acp",
        "extra": { "selected_session_mcp_servers": [{
            "id": 3,
            "name": "search",
            "transport": { "type": "stdio", "command": "echo" }
        }] }
    })).unwrap();
    let response = svc.create("user_1", req).await.unwrap();
    assert_eq!(response.extra["session_mcp_servers"][0]["id"], "3");
}
```

Also deserialize a boolean `id` and assert it returns an error.

- [ ] **Step 2: Run tests to verify the numeric compatibility case fails**

Run: `cargo test -p nomifun-conversation create_normalizes_legacy_numeric_session_mcp_ids_to_strings --lib`

Expected: FAIL with `Invalid selected_session_mcp_servers` and `expected a string`.

- [ ] **Step 3: Write minimal implementation**

```rust
#[serde(deserialize_with = "deserialize_session_mcp_id")]
pub id: String,
```

Implement `deserialize_session_mcp_id` with an untagged enum containing only `String` and `i64`, returning `id.to_string()` for the integer variant.

- [ ] **Step 4: Run focused tests to verify both behaviors**

Run: `cargo test -p nomifun-conversation session_mcp --lib`

Expected: PASS; numeric IDs persist as strings and boolean IDs remain rejected.

### Task 3: Verify affected integration boundaries

**Files:**
- Verify only: `ui/src/renderer/pages/guid/hooks/useGuidSend.ts`
- Verify only: `crates/backend/nomifun-conversation/src/service.rs`
- Verify only: `crates/backend/nomifun-ai-agent/src/factory/acp.rs`
- Verify only: `crates/backend/nomifun-ai-agent/src/factory/nomi.rs`

**Interfaces:**
- Consumes: string session MCP identifier from conversation extra.
- Produces: an MCP snapshot that is valid for ACP and Nomi agent factories.

- [ ] **Step 1: Run frontend regression and type validation**

Run: `bun test ui/src/renderer/hooks/mcp/catalog.test.ts && cd ui && bun run typecheck`

Expected: PASS.

- [ ] **Step 2: Run complete affected Rust crate tests**

Run: `cargo test -p nomifun-api-types -p nomifun-conversation`

Expected: PASS.

- [ ] **Step 3: Run factory contract tests**

Run: `cargo test -p nomifun-ai-agent session_mcp`

Expected: PASS.
