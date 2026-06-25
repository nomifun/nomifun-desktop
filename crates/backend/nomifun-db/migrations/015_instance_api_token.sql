-- Single-row table holding the SHA-256 hash of the long-lived instance API
-- token used by the Remote capability front door (`/mcp`). All-or-nothing
-- trust: one token per instance, hash-only at rest, revocable by deleting the
-- row. CHECK(id = 1) enforces the singleton invariant.
CREATE TABLE IF NOT EXISTS instance_api_token (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    token_hash TEXT    NOT NULL,
    created_at INTEGER NOT NULL
);
