-- Host-owned Plugin Surface session authority.
--
-- The raw bearer capability is never persisted. The Host stores only its
-- digest and binds it to one owner, Plugin, Active Release, and epoch.
-- Opening or reloading replaces the exact session; Publish, Rollback,
-- Disable, Close, and delete paths revoke the row transactionally.

CREATE TABLE plugin_surface_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    surface_session_id TEXT NOT NULL UNIQUE CHECK (
        length(surface_session_id) = 36
        AND lower(surface_session_id) = surface_session_id
        AND surface_session_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(surface_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
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
    generation INTEGER NOT NULL CHECK (generation >= 1),
    capability_digest TEXT NOT NULL UNIQUE CHECK (
        length(capability_digest) = 64
        AND lower(capability_digest) = capability_digest
        AND capability_digest NOT GLOB '*[^0-9a-f]*'
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
    issued_at_ms INTEGER NOT NULL CHECK (issued_at_ms > 0),
    UNIQUE (owner_user_id, plugin_product_id),
    UNIQUE (owner_user_id, plugin_product_id, generation),
    UNIQUE (
        owner_user_id,
        plugin_product_id,
        capability_digest,
        active_release_id,
        active_release_digest,
        active_release_epoch
    )
);

CREATE INDEX idx_plugin_surface_sessions_owner_user_id
    ON plugin_surface_sessions(owner_user_id);
CREATE INDEX idx_plugin_surface_sessions_plugin_product_id
    ON plugin_surface_sessions(plugin_product_id);
CREATE INDEX idx_plugin_surface_sessions_active_release_id
    ON plugin_surface_sessions(active_release_id);
CREATE INDEX idx_plugin_surface_sessions_capability_digest
    ON plugin_surface_sessions(capability_digest);
