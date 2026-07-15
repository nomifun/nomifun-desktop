# MCP JSON Identifier Repair Design — Superseded

Status: historical and non-executable.

This design was written before ID-contract v2 and described an MCP entity-ID
shape that is no longer valid. The old boundary-coercion design has been
removed to prevent it from being reused.

A persisted MCP server is now identified by a strict `McpServerId`:
`mcp_{lowercase-hyphenated-UUIDv7}`. It is a JSON string, stored as SQLite
`TEXT`, and validated as the Rust newtype at input boundaries. Alternate JSON
types and compatibility fallbacks are rejected. Session-only or remotely
issued locators are separate opaque values and never substitute for this
entity ID.

The authoritative design is
[`../../architecture/id-system.md`](../../architecture/id-system.md).
