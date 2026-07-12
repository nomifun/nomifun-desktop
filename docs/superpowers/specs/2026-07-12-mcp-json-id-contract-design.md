# MCP JSON ID Contract Repair Design

## Problem

The MCP catalog migrated its persisted primary key and wire response from a
string to a JSON number. The migration updated `McpServerResponse.id`, the
frontend `IMcpServer.id`, and path-parameter callers, but left
`TestMcpConnectionRequest.id` as `Option<String>`. The frontend connection-test
hook sends the complete `IMcpServer` as the JSON body, so a persisted server
produces a body beginning with `{"id":1,...}`. Axum rejects that body before
the handler runs with `invalid type: integer 1, expected a string`.

The modal then appends a generic “check MCP JSON” hint, which incorrectly
blames the user-provided server configuration even though the invalid field was
the NomiFun-managed catalog id.

This is the same migration-failure family previously seen in AutoWork and IDMM:
frontend numeric session ids reached backend request DTO string fields. Those
fixes covered only `target_id`, so they did not protect the MCP request.

## Goals

- A saved MCP server can be connection-tested without JSON deserialization
  failure.
- The canonical MCP catalog id is numeric in the frontend, request DTO,
  service layer, and repository layer.
- Already-released clients that send a numeric string such as `"id":"1"`
  remain compatible.
- Non-numeric strings are rejected at the HTTP boundary rather than flowing
  into business logic.
- Regression tests cover the DTO boundary, the real HTTP route plus result
  persistence, and the frontend request mapping.
- The frontend no longer sends an entire response/storage entity as the body
  of the connection-test request.

## Considered Approaches

### 1. Convert the frontend id to a string

This is the smallest patch, but it preserves the wrong contract, does not help
already-released clients, and makes another number/string drift likely.

### 2. Keep the backend field as `String` and deserialize either shape

This restores compatibility, but the wrong type remains in the service layer
and must still be parsed before accessing the numeric repository key.

### 3. Canonical numeric id with boundary compatibility

This is the selected approach. The request field becomes `Option<i64>` and a
shared boundary deserializer accepts either a JSON integer or a decimal integer
string. The service receives `i64` directly. The current frontend sends a
dedicated request containing only `id`, `name`, and `transport`.

## Architecture and Data Flow

1. `useMcpConnection` passes an `IMcpServer` to a pure request builder.
2. The builder returns a dedicated `McpConnectionTestRequest` containing only
   `id`, `name`, and `transport`; the canonical id remains a number.
3. `mcpService.testMcpConnection` serializes that dedicated request.
4. `TestMcpConnectionRequest` deserializes `id: 1` and legacy `id: "1"` into
   `Option<i64>`.
5. The route runs the connection test and passes the numeric id directly to
   `McpConfigService::persist_test_result`.
6. The service updates status and tools through `IMcpServerRepository` without
   reparsing an id string.

Requests without an id remain valid because unsaved/detected MCP definitions
can still be tested without persistence.

## Components

### Shared numeric-id compatibility helper

`nomifun-api-types::serde_util` will expose an optional-i64 deserializer for
request-boundary compatibility. It accepts signed JSON integers and decimal
integer strings, preserves a missing or explicit-null field as `None`, and
rejects booleans, floating-point values, objects, arrays, and non-numeric
strings with a field-specific Serde error.

The existing opaque session-id helper remains separate: AutoWork and IDMM use a
string internally because one field can represent more than one session
domain. Numeric database primary keys must not reuse that opaque-string helper.

### MCP request and service contract

`TestMcpConnectionRequest.id` becomes `Option<i64>` and uses the compatibility
helper. `McpConfigService::persist_test_result` accepts `i64`; its callers and
tests stop converting ids to strings. Path parameters are unchanged because
Axum supplies them as strings and the existing service boundary parses them.

### Frontend request boundary

The frontend defines a `McpConnectionTestRequest` rather than using
`IMcpServer` as the request body type. A pure builder copies only the fields the
endpoint owns. This prevents response-only or local-state fields such as
`enabled`, `tools`, timestamps, `original_json`, and `last_test_status` from
silently becoming part of the API contract.

## Error Handling

- `id: 1` and `id: "1"` are accepted.
- A missing or null `id` means “test without persisting a catalog result.”
- A non-numeric string is rejected as an invalid numeric MCP server id before
  the connection attempt.
- Connection/protocol failures continue to use the existing structured MCP
  error codes and UI formatting.
- The generic MCP JSON hint is not changed in this focused repair because the
  internal id deserialization failure is eliminated; transport/configuration
  failures still benefit from the hint.

## Testing

### DTO tests

- Reproduce the screenshot body with numeric `id: 1` and verify successful
  deserialization.
- Verify legacy decimal string `id: "1"` becomes numeric `1`.
- Verify missing/null id remains `None`.
- Verify non-numeric string and floating-point id are rejected.

### Backend route/service tests

- Send the real authenticated HTTP endpoint a saved numeric server id and a
  deterministic failing transport.
- Verify the response reaches connection-test behavior rather than returning a
  JSON-body 400.
- Reload the saved row and verify the failure status was persisted for that
  numeric id.
- Keep existing service status/tool persistence tests green after removing
  string conversion.

### Frontend tests

- Verify the request builder preserves the numeric id.
- Verify it includes only `id`, `name`, and `transport`.
- Run the UI typecheck so the hook, bridge, and request DTO agree.

## Scope and Audit Result

The active failure is limited to the MCP connection-test request. The earlier
AutoWork and IDMM fields are deliberately opaque session handles and already
have explicit mixed-shape boundary handling. Other locally persisted numeric
entities found in the request audit use numeric body fields or path parameters.
The systemic repair here is therefore a canonical numeric request/service
contract plus an explicit compatibility boundary and three-layer regression
coverage, not a global coercion of every string field.
