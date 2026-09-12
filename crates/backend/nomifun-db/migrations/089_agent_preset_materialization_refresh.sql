-- A Preset revision digest covers the user-authored payload and capability
-- contracts. A compatible Plugin update can preserve that digest while a new
-- immutable revision freezes the updated Artifact provenance in its Snapshot.
-- Revision number remains the exact ordering identity; duplicate semantic
-- digests are therefore valid and required for safe materialization refresh.

CREATE TABLE nomi_agent_preset_revisions_v089 (
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
    payload_json             TEXT NOT NULL CHECK (
        json_valid(payload_json)
        AND json_type(payload_json) = 'object'
    ),
    revision_digest          TEXT NOT NULL CHECK (
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
    reason                   TEXT NOT NULL,
    snapshot_json            TEXT NOT NULL CHECK (
        json_valid(snapshot_json)
        AND json_type(snapshot_json) = 'object'
    ),
    contribution_locks_json  TEXT NOT NULL DEFAULT '[]' CHECK (
        json_valid(contribution_locks_json)
        AND json_type(contribution_locks_json) = 'array'
    ),
    UNIQUE (preset_id, revision_no)
);

INSERT INTO nomi_agent_preset_revisions_v089 (
    id,
    revision_id,
    preset_id,
    revision_no,
    schema_version,
    payload_json,
    revision_digest,
    created_by,
    created_at,
    reason,
    snapshot_json,
    contribution_locks_json
)
SELECT
    id,
    revision_id,
    preset_id,
    revision_no,
    schema_version,
    payload_json,
    revision_digest,
    created_by,
    created_at,
    reason,
    snapshot_json,
    contribution_locks_json
FROM nomi_agent_preset_revisions;

DROP TABLE nomi_agent_preset_revisions;
ALTER TABLE nomi_agent_preset_revisions_v089 RENAME TO nomi_agent_preset_revisions;

CREATE INDEX idx_nomi_agent_preset_revisions_preset_id
    ON nomi_agent_preset_revisions(preset_id, revision_no);
CREATE INDEX idx_nomi_agent_preset_revisions_created_by
    ON nomi_agent_preset_revisions(created_by);
