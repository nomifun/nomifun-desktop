-- Phase M1-0 owner-scoped Host KV persistence.
--
-- Config and Credential references already live on the clean-start Product
-- root introduced by migration 072. This migration adds only the Host KV
-- cells required by UI-only MiniApps and the later Service Host adapter. It
-- does not inspect, copy, alias, or mutate the retired `miniapps` store or any
-- existing M1 row. Files, Private SQLite, and Service Host state remain out of
-- scope.

CREATE TABLE miniapp_kv (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    miniapp_id TEXT NOT NULL CHECK (
        length(miniapp_id) = 36
        AND lower(miniapp_id) = miniapp_id
        AND miniapp_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(miniapp_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    namespace TEXT NOT NULL CHECK (
        length(namespace) BETWEEN 1 AND 128
        AND namespace NOT GLOB '*[^!-~]*'
    ),
    key TEXT NOT NULL CHECK (
        length(key) BETWEEN 1 AND 256
        AND key NOT GLOB '*[^!-~]*'
    ),
    value_json TEXT NOT NULL CHECK (json_valid(value_json)),
    revision INTEGER NOT NULL DEFAULT 1 CHECK (revision >= 1),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (owner_user_id, miniapp_id, namespace, key)
);

CREATE INDEX idx_miniapp_kv_owner_user_id
    ON miniapp_kv(owner_user_id);
CREATE INDEX idx_miniapp_kv_miniapp_id
    ON miniapp_kv(miniapp_id);
CREATE INDEX idx_miniapp_kv_product
    ON miniapp_kv(owner_user_id, miniapp_id, namespace, key);
