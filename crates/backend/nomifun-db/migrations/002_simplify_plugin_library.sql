-- Keep the published baseline receipt intact. Plugins now retain only their
-- current code and data; pending mutation snapshots still recover failed saves.
-- The startup migration owner disables foreign keys on its dedicated connection
-- before sqlx starts this transaction; DDL, copies and receipt remain atomic.
PRAGMA legacy_alter_table = ON;

DROP TRIGGER trg_plugin_mutations_identity_immutable;

CREATE TABLE plugin_mutations_next (
    mutation_id             TEXT PRIMARY KEY CHECK (
        length(mutation_id) = 36
        AND lower(mutation_id) = mutation_id
        AND mutation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mutation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id           TEXT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE CHECK (
        length(owner_user_id) = 36 AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    plugin_id               TEXT NOT NULL CHECK (
        length(plugin_id) = 36
        AND lower(plugin_id) = plugin_id
        AND plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    kind                    TEXT NOT NULL CHECK (kind IN ('install', 'update', 'permanent_delete')),
    phase                   TEXT NOT NULL CHECK (phase IN ('staging', 'prepared', 'committed', 'rolling_back', 'failed')),
    old_artifact_digest     TEXT CHECK (
        old_artifact_digest IS NULL OR (
            length(old_artifact_digest) = 64
            AND lower(old_artifact_digest) = old_artifact_digest
            AND old_artifact_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    new_artifact_digest     TEXT CHECK (
        new_artifact_digest IS NULL OR (
            length(new_artifact_digest) = 64
            AND lower(new_artifact_digest) = new_artifact_digest
            AND new_artifact_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    old_data_generation     TEXT CHECK (
        old_data_generation IS NULL OR (
            length(old_data_generation) = 36
            AND lower(old_data_generation) = old_data_generation
            AND old_data_generation GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(old_data_generation, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    old_config_json         TEXT CHECK (
        old_config_json IS NULL OR (
            json_valid(old_config_json)
            AND json_type(old_config_json) = 'object'
        )
    ),
    old_credential_bindings_json TEXT CHECK (
        old_credential_bindings_json IS NULL OR (
            json_valid(old_credential_bindings_json)
            AND json_type(old_credential_bindings_json) = 'array'
        )
    ),
    old_grants_json         TEXT CHECK (
        old_grants_json IS NULL OR (
            json_valid(old_grants_json)
            AND json_type(old_grants_json) = 'array'
        )
    ),
    new_data_generation     TEXT CHECK (
        new_data_generation IS NULL OR (
            length(new_data_generation) = 36
            AND lower(new_data_generation) = new_data_generation
            AND new_data_generation GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(new_data_generation, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    expected_revision       INTEGER CHECK (expected_revision IS NULL OR expected_revision > 0),
    error                   TEXT,
    created_at_ms           INTEGER NOT NULL CHECK (created_at_ms > 0),
    updated_at_ms           INTEGER NOT NULL CHECK (updated_at_ms > 0), draft_association_json TEXT CHECK (
    draft_association_json IS NULL OR (
        json_valid(draft_association_json) AND json_type(draft_association_json) = 'object'
        AND json_type(draft_association_json, '$.draft_id') IS 'text'
        AND length(json_extract(draft_association_json, '$.draft_id')) = 36
        AND lower(json_extract(draft_association_json, '$.draft_id')) = json_extract(draft_association_json, '$.draft_id')
        AND json_extract(draft_association_json, '$.draft_id') GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(json_extract(draft_association_json, '$.draft_id'), '-', '') NOT GLOB '*[^0-9a-f]*'
        AND json_type(draft_association_json, '$.expected_revision') IS 'integer'
        AND json_extract(draft_association_json, '$.expected_revision') > 0
    )
),
    UNIQUE (owner_user_id, plugin_id)
);

INSERT INTO plugin_mutations_next (mutation_id, owner_user_id, plugin_id, kind, phase, old_artifact_digest, new_artifact_digest, old_data_generation, old_config_json, old_credential_bindings_json, old_grants_json, new_data_generation, expected_revision, error, created_at_ms, updated_at_ms, draft_association_json)
SELECT mutation_id, owner_user_id, plugin_id, CASE WHEN kind = 'restore' THEN 'update' ELSE kind END, phase, old_artifact_digest, new_artifact_digest, old_data_generation, old_config_json, old_credential_bindings_json, old_grants_json, new_data_generation, expected_revision, error, created_at_ms, updated_at_ms, draft_association_json FROM plugin_mutations;

DROP TABLE plugin_mutations;
ALTER TABLE plugin_mutations_next RENAME TO plugin_mutations;

CREATE INDEX idx_plugin_mutations_recovery ON plugin_mutations(phase, created_at_ms, mutation_id);

CREATE TRIGGER trg_plugin_mutations_identity_immutable
BEFORE UPDATE ON plugin_mutations
WHEN NEW.mutation_id IS NOT OLD.mutation_id
  OR NEW.owner_user_id IS NOT OLD.owner_user_id
  OR NEW.plugin_id IS NOT OLD.plugin_id
  OR NEW.kind IS NOT OLD.kind
  OR NEW.old_artifact_digest IS NOT OLD.old_artifact_digest
  OR NEW.new_artifact_digest IS NOT OLD.new_artifact_digest
  OR NEW.old_data_generation IS NOT OLD.old_data_generation
  OR NEW.old_config_json IS NOT OLD.old_config_json
  OR NEW.old_credential_bindings_json IS NOT OLD.old_credential_bindings_json
  OR NEW.old_grants_json IS NOT OLD.old_grants_json
  OR NEW.new_data_generation IS NOT OLD.new_data_generation
  OR NEW.expected_revision IS NOT OLD.expected_revision
  OR NEW.created_at_ms IS NOT OLD.created_at_ms
  OR NEW.updated_at_ms < OLD.updated_at_ms
BEGIN
    SELECT RAISE(ABORT, 'Plugin mutation identity is immutable and time is monotonic');
END;

DROP TRIGGER trg_plugin_drafts_clear_base_before_plugin_delete;

DROP TRIGGER trg_plugins_identity_immutable;

DROP TRIGGER trg_plugins_revision_monotonic;

CREATE TABLE plugins_next (
    plugin_id                    TEXT PRIMARY KEY CHECK (
        length(plugin_id) = 36
        AND lower(plugin_id) = plugin_id
        AND plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id                TEXT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE CHECK (
        length(owner_user_id) = 36 AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    package_id                   TEXT NOT NULL CHECK (
        length(package_id) BETWEEN 1 AND 160
        AND package_id = lower(package_id)
        AND package_id NOT GLOB '*[^a-z0-9._-]*'
    ),
    name                         TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 160),
    description                  TEXT NOT NULL CHECK (length(description) BETWEEN 1 AND 4096),
    enabled                      INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    trashed_at_ms                INTEGER CHECK (trashed_at_ms IS NULL OR trashed_at_ms > 0),
    active_artifact_digest       TEXT NOT NULL REFERENCES plugin_artifacts(artifact_digest) ON DELETE RESTRICT,
    data_generation              TEXT NOT NULL CHECK (
        length(data_generation) = 36
        AND lower(data_generation) = data_generation
        AND data_generation GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(data_generation, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    revision                     INTEGER NOT NULL CHECK (revision > 0),
    config_json                  TEXT NOT NULL CHECK (json_valid(config_json) AND json_type(config_json) = 'object'),
    last_error                   TEXT,
    created_at_ms                INTEGER NOT NULL CHECK (created_at_ms > 0),
    updated_at_ms                INTEGER NOT NULL CHECK (updated_at_ms > 0),
    UNIQUE (owner_user_id, package_id),
    CHECK (trashed_at_ms IS NULL OR enabled = 0)
);

INSERT INTO plugins_next (plugin_id, owner_user_id, package_id, name, description, enabled, trashed_at_ms, active_artifact_digest, data_generation, revision, config_json, last_error, created_at_ms, updated_at_ms)
SELECT plugin_id, owner_user_id, package_id, name, description, enabled, trashed_at_ms, active_artifact_digest, data_generation, revision, config_json, last_error, created_at_ms, updated_at_ms FROM plugins;

DROP TABLE plugins;
ALTER TABLE plugins_next RENAME TO plugins;

CREATE INDEX idx_plugins_active_artifact ON plugins(active_artifact_digest);

CREATE INDEX idx_plugins_owner_updated ON plugins(owner_user_id, updated_at_ms DESC, plugin_id);

CREATE TRIGGER trg_plugin_drafts_clear_base_before_plugin_delete
BEFORE DELETE ON plugins
BEGIN
    UPDATE plugin_drafts SET base_revision = NULL, revision = revision + 1 WHERE plugin_id = OLD.plugin_id;
END;

CREATE TRIGGER trg_plugins_identity_immutable
BEFORE UPDATE ON plugins
WHEN NEW.plugin_id IS NOT OLD.plugin_id
  OR NEW.owner_user_id IS NOT OLD.owner_user_id
  OR NEW.package_id IS NOT OLD.package_id
  OR NEW.created_at_ms IS NOT OLD.created_at_ms
BEGIN
    SELECT RAISE(ABORT, 'Plugin identity is immutable');
END;

CREATE TRIGGER trg_plugins_revision_monotonic
BEFORE UPDATE ON plugins
WHEN NEW.revision <> OLD.revision + 1
  OR NEW.updated_at_ms < OLD.updated_at_ms
BEGIN
    SELECT RAISE(ABORT, 'Plugin revision must advance exactly once');
END;

-- Abort the whole replacement if any existing relationship was lost.
DROP TRIGGER trg_plugin_drafts_identity_immutable;
ALTER TABLE plugin_drafts DROP COLUMN imported_context_json;
CREATE TRIGGER trg_plugin_drafts_identity_immutable
BEFORE UPDATE ON plugin_drafts
WHEN NEW.draft_id IS NOT OLD.draft_id OR NEW.owner_user_id IS NOT OLD.owner_user_id
    OR NEW.created_at_ms IS NOT OLD.created_at_ms OR NEW.updated_at_ms < OLD.updated_at_ms
BEGIN
    SELECT RAISE(ABORT, 'Plugin Draft identity and time are immutable');
END;
UPDATE plugin_drafts
SET verification_json = json_remove(verification_json, '$.approval')
WHERE json_type(verification_json, '$.approval') IS NOT NULL;

CREATE TEMP TABLE plugin_schema_integrity (violations INTEGER CHECK (violations = 0));
INSERT INTO plugin_schema_integrity SELECT COUNT(*) FROM pragma_foreign_key_check;
DROP TABLE plugin_schema_integrity;
PRAGMA legacy_alter_table = OFF;
