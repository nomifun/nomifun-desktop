-- Mini-apps: AI-generated, self-contained single-file web tools, solidified out
-- of a conversation and reopened instantly from the sidebar library.
--
-- A `type='nomi'` conversation started in mini-app builder mode writes one
-- `miniapp.html` into its workspace; solidifying stores that document's full
-- text here, so running the app later needs neither the conversation nor the
-- workspace. The body is handed out by an auth-exempt
-- `GET /api/miniapps/{miniapp_id}/serve` and loaded as an iframe `src`.
--
-- Invariants (verified against id_schema_contract on every boot):
--   * `id INTEGER PRIMARY KEY AUTOINCREMENT` + a bare UUIDv7 business id with
--     the standard GLOB/length/lowercase CHECK.
--   * No physical FOREIGN KEY; `user_id` is a logical reference (Cascade) and
--     `source_conversation_id` a nullable one (SetNull) — the app outlives the
--     conversation that produced it, so deleting that conversation forgets the
--     provenance link and nothing else.
--   * The HTML body lives inline in this table instead of under `{data_dir}`:
--     one self-contained document per row needs no path guard, no backup-root
--     registration and no orphaned-file sweeper.
--   * `html_size` is stored, not derived, so no metadata read ever touches the
--     bodies; the repository is the single writer and keeps it in step with
--     `html`.
--
-- Verification performed: `cargo test -p nomifun-db --test miniapps_schema`
-- boots an in-memory DB, applies this migration and runs
-- validate_id_schema_contract; `cargo test -p nomifun-app --test miniapp_e2e`
-- exercises owner-scoped CRUD plus the unauthenticated serve route.

CREATE TABLE miniapps (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    miniapp_id             TEXT NOT NULL UNIQUE
                           CHECK (
                               length(miniapp_id) = 36
                               AND lower(miniapp_id) = miniapp_id
                               AND miniapp_id GLOB '????????-????-7???-[89ab]???-????????????'
                               AND replace(miniapp_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                           ),
    user_id                TEXT NOT NULL,
    name                   TEXT NOT NULL,
    description            TEXT NOT NULL DEFAULT '',
    -- Optional emoji or very short label the library grid renders.
    icon                   TEXT,
    -- The complete self-contained HTML document (inline CSS/JS).
    html                   TEXT NOT NULL,
    -- Byte length of `html`, maintained by the writer on every body write.
    -- Persisted rather than derived: metadata reads are the common case (the
    -- library grid lists every app the owner has), and computing
    -- `length(CAST(html AS BLOB))` per row would make a list request scan every
    -- document's bytes — proportional to the total bytes ever solidified.
    html_size              INTEGER NOT NULL,
    -- Nullable: the conversation this app was solidified from, when there was one.
    source_conversation_id TEXT,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL,
    -- user_id is a logical reference to users.user_id (CanonicalUuidV7), so the
    -- column carries the same UUIDv7 CHECK the contract requires (mirrors
    -- ssh_hosts.user_id).
    CHECK (length(user_id) = 36 AND lower(user_id) = user_id AND user_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(user_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    -- Same value contract for the nullable provenance reference. NULL passes:
    -- an app solidified outside any conversation has no source to name, and a
    -- deleted source conversation walks this column back to NULL.
    CHECK (source_conversation_id IS NULL OR (length(source_conversation_id) = 36 AND lower(source_conversation_id) = source_conversation_id AND source_conversation_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(source_conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'))
);

CREATE INDEX idx_miniapps_user_id ON miniapps(user_id);
CREATE INDEX idx_miniapps_source_conversation_id ON miniapps(source_conversation_id);
