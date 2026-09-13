-- Plugin Runtime owner-scoped Host KV persistence.
--
-- Config and Credential references already live on the clean-start Product
-- root introduced by migration 072. This migration adds only the Host KV
-- cells required by UI-only Plugins and the later Service Host adapter. It
-- does not inspect, copy, alias, or mutate the retired `plugins` store or any
-- existing M1 row. Files, Private SQLite, and Service Host state remain out of
-- scope.

CREATE TABLE plugin_kv (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_product_id TEXT NOT NULL CHECK (
        length(plugin_product_id) = 36
        AND lower(plugin_product_id) = plugin_product_id
        AND plugin_product_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_product_id, '-', '') NOT GLOB '*[^0-9a-f]*'
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
    UNIQUE (owner_user_id, plugin_product_id, namespace, key)
);

CREATE INDEX idx_plugin_kv_owner_user_id
    ON plugin_kv(owner_user_id);
CREATE INDEX idx_plugin_kv_plugin_product_id
    ON plugin_kv(plugin_product_id);
CREATE INDEX idx_plugin_kv_product
    ON plugin_kv(owner_user_id, plugin_product_id, namespace, key);
