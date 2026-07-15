# Session MCP Identifier Contract Design — Superseded

Status: historical and non-executable.

The earlier design normalized configured MCP identifiers between incompatible
wire shapes. ID-contract v2 replaces that design: a persisted configured MCP
server is referenced only by its canonical `McpServerId` JSON string, while a
session-only or external locator uses a separate purpose-specific opaque
field. Neither category is converted into the other.

The obsolete implementation and compatibility examples were removed so this
document cannot be mistaken for current guidance. See the authoritative
[`../../architecture/id-system.md`](../../architecture/id-system.md).
