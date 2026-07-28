-- Converge per-model metadata (providers.models + 5 parallel JSON maps +
-- model_profiles) into one authoritative provider_models entity table, and add
-- provider_connections for non-default per-task connection profiles (e.g. a
-- separate voice domain + credential set). The providers row itself remains
-- the 'default' connection in P0; model_profiles is dropped by migration 015
-- after the Rust read path switches.

CREATE TABLE provider_models (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id       TEXT NOT NULL,
    model             TEXT NOT NULL,
    enabled           INTEGER NOT NULL DEFAULT 1,
    sort_order        INTEGER NOT NULL DEFAULT 0,
    tasks             TEXT NOT NULL DEFAULT '[]',
    traits            TEXT NOT NULL DEFAULT '[]',
    protocol          TEXT,
    connection_role   TEXT,
    params            TEXT NOT NULL DEFAULT '{}',
    context_limit     INTEGER,
    description       TEXT,
    source            TEXT NOT NULL DEFAULT 'inferred',
    health            TEXT,
    health_checked_at INTEGER,
    created_at        INTEGER NOT NULL,
    updated_at        INTEGER NOT NULL,
    UNIQUE (provider_id, model),
    CHECK (length(provider_id) = 36 AND lower(provider_id) = provider_id AND provider_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE INDEX idx_provider_models_provider_id ON provider_models(provider_id);

CREATE TABLE provider_connections (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    connection_id         TEXT NOT NULL UNIQUE
                          CHECK (length(connection_id) = 36 AND lower(connection_id) = connection_id AND connection_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(connection_id, '-', '') NOT GLOB '*[^0-9a-f]*'),
    provider_id           TEXT NOT NULL,
    role                  TEXT NOT NULL,
    label                 TEXT,
    base_url              TEXT NOT NULL,
    auth_scheme           TEXT NOT NULL DEFAULT 'bearer',
    credentials_encrypted TEXT NOT NULL,
    is_full_url           INTEGER NOT NULL DEFAULT 0,
    extra                 TEXT NOT NULL DEFAULT '{}',
    created_at            INTEGER NOT NULL,
    updated_at            INTEGER NOT NULL,
    UNIQUE (provider_id, role),
    CHECK (length(provider_id) = 36 AND lower(provider_id) = provider_id AND provider_id GLOB '????????-????-7???-[89ab]???-????????????' AND replace(provider_id, '-', '') NOT GLOB '*[^0-9a-f]*')
);

CREATE INDEX idx_provider_connections_provider_id ON provider_connections(provider_id);

-- Backfill: one provider_models row per (provider, catalog model). Profile
-- fields merge from model_profiles when present; per-model map values merge
-- from the providers row's JSON map columns. Orphan model_profiles rows (their
-- model no longer in providers.models) are intentionally NOT migrated — that
-- is the orphan cleanup. Idempotency guard keeps this statement re-runnable.
INSERT INTO provider_models (
    provider_id, model, enabled, sort_order, tasks, traits, protocol, params,
    context_limit, description, source, health, created_at, updated_at
)
SELECT
    p.provider_id,
    je.value,
    COALESCE((SELECT e.value FROM json_each(COALESCE(p.model_enabled, '{}')) e WHERE e.key = je.value), 1),
    je.key,
    COALESCE(mp.tasks, '[]'),
    COALESCE(mp.traits, '[]'),
    (SELECT e.value FROM json_each(COALESCE(p.model_protocols, '{}')) e WHERE e.key = je.value),
    COALESCE(mp.params, '{}'),
    (SELECT e.value FROM json_each(COALESCE(p.model_context_limits, '{}')) e WHERE e.key = je.value),
    (SELECT e.value FROM json_each(COALESCE(p.model_descriptions, '{}')) e WHERE e.key = je.value),
    COALESCE(mp.source, 'inferred'),
    (SELECT json(e.value) FROM json_each(COALESCE(p.model_health, '{}')) e WHERE e.key = je.value),
    CAST(strftime('%s', 'now') AS INTEGER) * 1000,
    COALESCE(mp.updated_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000)
FROM providers p
JOIN json_each(p.models) je
LEFT JOIN model_profiles mp
    ON mp.provider_id = p.provider_id AND mp.model = je.value
WHERE NOT EXISTS (
    SELECT 1 FROM provider_models pm
    WHERE pm.provider_id = p.provider_id AND pm.model = je.value
);
