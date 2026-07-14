# Session MCP Identifier Contract Design

## Goal

Prevent MCP selection from failing during conversation creation when a selected server has an integer catalog identifier, while preserving compatibility with already deployed clients.

## Root cause

The catalog model uses an integer database primary key (`IMcpServer.id`). The frontend session-snapshot type was derived with `Pick<IMcpServer, ...>`, so the request serializes `selected_session_mcp_servers[].id` as a number. The backend snapshot DTO intentionally defines the identifier as `String`, because it represents both repository-backed and session-only MCP servers. Strict deserialization therefore rejects the request before session creation.

## Design

- Define the frontend session snapshot as its own interface with `id: string`; it must not inherit the catalog entity's identifier type.
- Normalize the identifier at the catalog-to-session boundary with `String(server.id)`.
- Preserve a strict server object and transport schema. On the backend, accept only string or integer values for `SessionMcpServer.id`, then normalize integers to strings during deserialization for rolling-upgrade compatibility.
- Align the frontend conversation MCP-status identifier to `string`, matching the response DTO and avoiding a later ID-comparison mismatch.

## Data flow

`IMcpServer (id: number)` -> `toSessionMcpServer (id: string)` -> `extra.selected_session_mcp_servers` -> backend compatibility parser -> `SessionMcpServer (id: String)` -> persisted `extra.session_mcp_servers` -> Agent factory.

## Error handling

Non-string, non-integer identifiers remain invalid requests. The compatibility rule is deliberately scoped to this one identifier; it does not weaken validation for MCP names, transports, or other request fields.

## Verification

- A frontend unit test asserts that the session snapshot stringifies a numeric catalog ID.
- A backend unit test proves a legacy numeric ID deserializes and is persisted as a string.
- A backend unit test proves malformed ID types remain rejected.
- Targeted frontend and Rust tests, frontend typecheck, and Rust crate tests validate the full boundary contract.
