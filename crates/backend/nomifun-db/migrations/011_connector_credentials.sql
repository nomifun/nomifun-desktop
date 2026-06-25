-- Migration 010: encrypted credentials for source connectors (feishu/notion/…).
--
-- Stores per-connector credentials as an opaque AES-256-GCM ciphertext blob
-- (`payload_encrypted`); the service layer holds the encryption key and the
-- JSON payload shape (e.g. `{ "app_id": ..., "app_secret": ... }`), exactly
-- like the providers table's `api_key_encrypted`. Multiple credentials of the
-- same kind are allowed (different tenants/accounts), so the key is a surrogate
-- id, not the kind.
--
-- Additive: never touches 001_baseline (checksum stability).
CREATE TABLE IF NOT EXISTS connector_credentials (
    id                TEXT PRIMARY KEY,
    kind              TEXT NOT NULL,
    name              TEXT NOT NULL,
    payload_encrypted TEXT NOT NULL,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_connector_credentials_kind ON connector_credentials(kind);
