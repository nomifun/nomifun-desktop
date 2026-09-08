-- Owner-scoped user authorization for strict UI-only MiniApp auto Publish.
--
-- The authorization is product state, not manifest/config JSON. Clearing an
-- authorization keeps the row with `enabled = 0` so stale grants cannot be
-- replayed after a later user decision. The repository owns all logical
-- references and CAS transitions; no foreign keys or triggers are used.

CREATE TABLE miniapp_publish_authorizations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    authorization_id TEXT NOT NULL UNIQUE CHECK (
        length(authorization_id) = 36
        AND lower(authorization_id) = authorization_id
        AND authorization_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(authorization_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    miniapp_id TEXT NOT NULL UNIQUE CHECK (
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
    revision INTEGER NOT NULL CHECK (revision >= 1),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    user_authorized_at_ms INTEGER NOT NULL CHECK (user_authorized_at_ms > 0),
    UNIQUE (owner_user_id, miniapp_id),
    UNIQUE (owner_user_id, miniapp_id, authorization_id)
);

CREATE INDEX idx_miniapp_publish_authorizations_owner_user_id
    ON miniapp_publish_authorizations(owner_user_id);
CREATE INDEX idx_miniapp_publish_authorizations_miniapp_id
    ON miniapp_publish_authorizations(miniapp_id);

-- Materialized Catalog projection for an enabled MiniApp. Disabled MiniApps
-- have no row. Publish, Rollback, Enable, and Disable update this projection
-- in the same transaction as Product and library state.

CREATE TABLE miniapp_catalog_publications (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    miniapp_id TEXT NOT NULL UNIQUE CHECK (
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
    UNIQUE (owner_user_id, miniapp_id),
    UNIQUE (
        owner_user_id,
        miniapp_id,
        active_release_id,
        active_release_digest,
        active_release_epoch
    )
);

CREATE INDEX idx_miniapp_catalog_publications_owner_user_id
    ON miniapp_catalog_publications(owner_user_id);
CREATE INDEX idx_miniapp_catalog_publications_miniapp_id
    ON miniapp_catalog_publications(miniapp_id);
CREATE INDEX idx_miniapp_catalog_publications_active_release_id
    ON miniapp_catalog_publications(active_release_id);

INSERT INTO miniapp_catalog_publications (
    miniapp_id,
    owner_user_id,
    active_release_id,
    active_release_digest,
    active_release_epoch,
    catalog_digest
)
SELECT
    miniapp_id,
    owner_user_id,
    active_release_id,
    active_release_digest,
    active_release_epoch,
    materialized_catalog_digest
FROM miniapp_products
WHERE lifecycle = 'enabled'
  AND active_release_id IS NOT NULL
  AND active_release_digest IS NOT NULL
  AND active_release_epoch > 0;
