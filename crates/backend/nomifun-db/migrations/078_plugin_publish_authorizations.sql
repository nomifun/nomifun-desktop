-- Owner-scoped user authorization for strict UI-only Plugin auto Publish.
--
-- The authorization is product state, not manifest/config JSON. Clearing an
-- authorization keeps the row with `enabled = 0` so stale grants cannot be
-- replayed after a later user decision. The repository owns all logical
-- references and CAS transitions; no foreign keys or triggers are used.

CREATE TABLE plugin_publish_authorizations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    authorization_id TEXT NOT NULL UNIQUE CHECK (
        length(authorization_id) = 36
        AND lower(authorization_id) = authorization_id
        AND authorization_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(authorization_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    plugin_product_id TEXT NOT NULL UNIQUE CHECK (
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
    revision INTEGER NOT NULL CHECK (revision >= 1),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    user_authorized_at_ms INTEGER NOT NULL CHECK (user_authorized_at_ms > 0),
    UNIQUE (owner_user_id, plugin_product_id),
    UNIQUE (owner_user_id, plugin_product_id, authorization_id)
);

CREATE INDEX idx_plugin_publish_authorizations_owner_user_id
    ON plugin_publish_authorizations(owner_user_id);
CREATE INDEX idx_plugin_publish_authorizations_plugin_product_id
    ON plugin_publish_authorizations(plugin_product_id);

-- Materialized Catalog projection for an enabled Plugin. Disabled Plugins
-- have no row. Publish, Rollback, Enable, and Disable update this projection
-- in the same transaction as Product and library state.

CREATE TABLE plugin_catalog_publications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    plugin_product_id TEXT NOT NULL UNIQUE CHECK (
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
    active_release_id TEXT NOT NULL CHECK (
        length(active_release_id) = 36
        AND lower(active_release_id) = active_release_id
        AND active_release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(active_release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    active_release_digest TEXT NOT NULL CHECK (
        length(active_release_digest) = 64
        AND lower(active_release_digest) = active_release_digest
        AND active_release_digest NOT GLOB '*[^0-9a-f]*'
    ),
    active_release_epoch INTEGER NOT NULL CHECK (active_release_epoch > 0),
    catalog_digest TEXT NOT NULL CHECK (
        length(catalog_digest) = 64
        AND lower(catalog_digest) = catalog_digest
        AND catalog_digest NOT GLOB '*[^0-9a-f]*'
    ),
    UNIQUE (owner_user_id, plugin_product_id),
    UNIQUE (
        owner_user_id,
        plugin_product_id,
        active_release_id,
        active_release_digest,
        active_release_epoch
    )
);

CREATE INDEX idx_plugin_catalog_publications_owner_user_id
    ON plugin_catalog_publications(owner_user_id);
CREATE INDEX idx_plugin_catalog_publications_plugin_product_id
    ON plugin_catalog_publications(plugin_product_id);
CREATE INDEX idx_plugin_catalog_publications_active_release_id
    ON plugin_catalog_publications(active_release_id);

INSERT INTO plugin_catalog_publications (
    plugin_product_id,
    owner_user_id,
    active_release_id,
    active_release_digest,
    active_release_epoch,
    catalog_digest
)
SELECT
    plugin_product_id,
    owner_user_id,
    active_release_id,
    active_release_digest,
    active_release_epoch,
    materialized_catalog_digest
FROM plugin_products
WHERE lifecycle = 'enabled'
  AND active_release_id IS NOT NULL
  AND active_release_digest IS NOT NULL
  AND active_release_epoch > 0;
