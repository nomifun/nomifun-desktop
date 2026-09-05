-- Nomi-core Agent Settings control-plane facts.
--
-- These tables are deliberately distinct from the Fresh-v4-only
-- `agent_sessions` event store.  The current product continues to execute
-- through the Nomi Conversation owner; the tables below persist the canonical
-- preset/revision/snapshot/binding facts consumed by that owner without
-- creating a second runtime or session aggregate.

CREATE TABLE nomi_agent_presets (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    preset_id                TEXT NOT NULL UNIQUE CHECK (
        length(preset_id) = 36
        AND lower(preset_id) = preset_id
        AND preset_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(preset_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id            TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    source_kind              TEXT NOT NULL CHECK (source_kind = 'user'),
    display_name             TEXT NOT NULL CHECK (length(trim(display_name)) > 0),
    description              TEXT,
    current_revision         INTEGER CHECK (
        current_revision IS NULL OR current_revision >= 1
    ),
    created_at               INTEGER NOT NULL
);

CREATE TABLE nomi_agent_preset_revisions (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    revision_id              TEXT NOT NULL UNIQUE CHECK (length(trim(revision_id)) > 0),
    preset_id                TEXT NOT NULL CHECK (
        length(preset_id) = 36
        AND lower(preset_id) = preset_id
        AND preset_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(preset_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    revision_no              INTEGER NOT NULL CHECK (revision_no >= 1),
    schema_version           TEXT NOT NULL CHECK (length(trim(schema_version)) > 0),
    editor_document_json     TEXT NOT NULL CHECK (
        json_valid(editor_document_json)
        AND json_type(editor_document_json) = 'object'
    ),
    revision_digest           TEXT NOT NULL CHECK (
        length(revision_digest) = 64
        AND lower(revision_digest) = revision_digest
        AND revision_digest NOT GLOB '*[^0-9a-f]*'
    ),
    created_by               TEXT NOT NULL CHECK (
        length(created_by) = 36
        AND lower(created_by) = created_by
        AND created_by GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(created_by, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    created_at               INTEGER NOT NULL,
    reason                  TEXT NOT NULL,
    snapshot_json            TEXT NOT NULL CHECK (
        json_valid(snapshot_json)
        AND json_type(snapshot_json) = 'object'
    ),
    UNIQUE (preset_id, revision_no),
    UNIQUE (preset_id, revision_digest)
);

CREATE TABLE nomi_agent_bindings (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    target_kind              TEXT NOT NULL CHECK (length(trim(target_kind)) > 0),
    target_id                TEXT NOT NULL CHECK (length(trim(target_id)) > 0),
    owner_user_id            TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    agent_binding_json       TEXT NOT NULL CHECK (
        json_valid(agent_binding_json)
        AND json_type(agent_binding_json) = 'object'
    ),
    UNIQUE (target_kind, target_id)
);

CREATE INDEX idx_nomi_agent_presets_owner_user_id
    ON nomi_agent_presets(owner_user_id);
CREATE INDEX idx_nomi_agent_preset_revisions_preset_id
    ON nomi_agent_preset_revisions(preset_id, revision_no);
CREATE INDEX idx_nomi_agent_preset_revisions_created_by
    ON nomi_agent_preset_revisions(created_by);
CREATE INDEX idx_nomi_agent_bindings_owner_user_id
    ON nomi_agent_bindings(owner_user_id);
CREATE INDEX idx_nomi_agent_bindings_target
    ON nomi_agent_bindings(target_kind, target_id);
