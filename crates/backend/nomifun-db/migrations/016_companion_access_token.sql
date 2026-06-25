-- Per-companion access tokens for the Remote capability front door.
-- Replaces the singleton `instance_api_token` (015): every external connection
-- binds to exactly one companion. Only the SHA-256 hash is stored; the plaintext
-- is shown once at mint time and never persisted.
CREATE TABLE IF NOT EXISTS companion_access_token (
    companion_id TEXT PRIMARY KEY,
    token_hash   TEXT NOT NULL,
    created_at   INTEGER NOT NULL
);

-- Retire the singleton instance token. Safe whether or not 015 was ever applied
-- on this DB (fresh branch DB: 015 creates it, 016 drops it; already-migrated DB:
-- 016 drops the existing table). No dead table remains.
DROP TABLE IF EXISTS instance_api_token;
