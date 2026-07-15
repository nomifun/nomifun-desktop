# Session MCP Identifier Contract Plan — Superseded

Status: historical and non-executable.

This plan predates ID-contract v2. It proposed converting between multiple
representations of a configured MCP server identifier. That approach is now
forbidden, and its implementation examples have been removed.

Current code must preserve the distinction between:

- a persisted configured-server reference, which is a strict `McpServerId`
  (`mcp_{lowercase-hyphenated-UUIDv7}`) serialized as a JSON string; and
- a session-only or externally issued locator, which is an explicitly named
  opaque external/operation key and is not a NomiFun entity ID.

No session snapshot may stringify, parse, default, or otherwise coerce an
entity ID. Boundary tests must reject JSON numbers and malformed or
wrong-prefix strings. See
[`../../architecture/id-system.md`](../../architecture/id-system.md).
