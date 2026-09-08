-- Make MiniApp Host KV keys durable identities.
--
-- A deleted key is retained as a tombstone instead of being physically
-- removed. `revision` is the monotonic mutation cursor used by CAS, while
-- `key_generation` advances at the live -> tombstone boundary and therefore
-- fences a later incarnation of the same logical key. Existing rows are live
-- generation 1 entries.
--
-- This migration is intentionally additive. It does not edit migrations
-- 075-080 and does not introduce physical foreign keys or triggers.

ALTER TABLE miniapp_kv
    ADD COLUMN key_generation INTEGER NOT NULL DEFAULT 1
        CHECK (key_generation >= 1);

ALTER TABLE miniapp_kv
    ADD COLUMN is_tombstone INTEGER NOT NULL DEFAULT 0
        CHECK (is_tombstone IN (0, 1));
