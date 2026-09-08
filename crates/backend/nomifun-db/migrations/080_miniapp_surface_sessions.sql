-- Host-owned MiniApp Surface session authority.
--
-- The raw bearer capability is never persisted. The Host stores only its
-- digest and binds it to one owner, MiniApp, Active Release, and epoch.
-- Opening or reloading replaces the exact session; Publish, Rollback,
-- Disable, Close, and delete paths revoke the row transactionally.

CREATE TABLE miniapp_surface_sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    surface_session_id TEXT NOT NULL UNIQUE CHECK (
        length(surface_session_id) = 36
        AND lower(surface_session_id) = surface_session_id
        AND surface_session_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(surface_session_id, '-', '') NOT GLOB '*[^0-9a-f]*'
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
    UNIQUE (owner_user_id, miniapp_id),
    UNIQUE (owner_user_id, miniapp_id, generation),
    UNIQUE (
        owner_user_id,
        miniapp_id,
        capability_digest,
        active_release_id,
        active_release_digest,
        active_release_epoch
    )
);

CREATE INDEX idx_miniapp_surface_sessions_owner_user_id
    ON miniapp_surface_sessions(owner_user_id);
CREATE INDEX idx_miniapp_surface_sessions_miniapp_id
    ON miniapp_surface_sessions(miniapp_id);
CREATE INDEX idx_miniapp_surface_sessions_active_release_id
    ON miniapp_surface_sessions(active_release_id);
CREATE INDEX idx_miniapp_surface_sessions_capability_digest
    ON miniapp_surface_sessions(capability_digest);
