# MCP JSON Identifier Repair Plan — Superseded

Status: historical and non-executable.

This plan predates ID-contract v2. Its proposed MCP catalog identifier
representation and compatibility coercion are forbidden by the current
contract, so the old implementation snippets have been removed rather than
left as copyable guidance.

The current rules are:

- a persisted configured MCP server uses `McpServerId` with the canonical
  `mcp_{lowercase-hyphenated-UUIDv7}` representation;
- the value is a JSON string and SQLite `TEXT` at every durable boundary;
- Rust request and response boundaries validate the typed ID strictly;
- a JSON number, decimal text masquerading as an entity ID, wrong prefix,
  malformed UUID, or missing-value fallback is rejected rather than coerced;
- an external or session-only MCP locator stays an explicitly named opaque
  external/operation key and must not be used as a configured-server entity ID.

See [`../../architecture/id-system.md`](../../architecture/id-system.md) for
the authoritative contract. Any new MCP boundary test must prove canonical
string round trips and rejection of alternate representations.
