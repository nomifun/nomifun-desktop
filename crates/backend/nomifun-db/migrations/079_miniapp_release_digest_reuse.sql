-- Allow multiple immutable Release records to reference the same
-- content-addressed Artifact digest.
--
-- Artifact bytes remain unique by artifact_digest. A Release row also carries
-- source/build lineage, so rebuilding identical bytes from a later Project
-- generation must be able to create a new Release identity without rewriting
-- the old row. This is a forward-only table rebuild; published migrations are
-- never edited in place.

CREATE TABLE miniapp_releases_v079_sequence (
    old_sequence INTEGER NOT NULL CHECK (old_sequence >= 0)
);

INSERT INTO miniapp_releases_v079_sequence (old_sequence)
SELECT COALESCE(
    (
        SELECT MAX(seq)
        FROM sqlite_sequence
        WHERE name = 'miniapp_releases'
    ),
    0
);

CREATE TABLE miniapp_releases_v079 (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    release_id TEXT NOT NULL UNIQUE CHECK (
        length(release_id) = 36
        AND lower(release_id) = release_id
        AND release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
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
    artifact_id TEXT NOT NULL CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest TEXT NOT NULL CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    manifest_digest TEXT NOT NULL CHECK (
        length(manifest_digest) = 64
        AND lower(manifest_digest) = manifest_digest
        AND manifest_digest NOT GLOB '*[^0-9a-f]*'
    ),
    release_digest TEXT NOT NULL CHECK (
        length(release_digest) = 64
        AND lower(release_digest) = release_digest
        AND release_digest NOT GLOB '*[^0-9a-f]*'
    ),
    origin_kind TEXT NOT NULL CHECK (origin_kind IN ('build', 'import')),
    origin_operation_id TEXT NOT NULL,
    source_kind TEXT NOT NULL CHECK (source_kind IN ('managed', 'runtime_only')),
    project_id TEXT,
    source_snapshot_digest TEXT,
    dependency_lock_digest TEXT,
    build_profile_version TEXT,
    build_generation INTEGER CHECK (build_generation IS NULL OR build_generation > 0),
    release_record_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(release_record_json)
               AND json_type(release_record_json) = 'object'),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    UNIQUE (owner_user_id, release_id),
    UNIQUE (owner_user_id, release_id, release_digest),
    CHECK (project_id IS NULL OR (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (
        length(origin_operation_id) = 36
        AND lower(origin_operation_id) = origin_operation_id
        AND origin_operation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(origin_operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    CHECK (
        (source_kind = 'managed'
         AND project_id IS NOT NULL
         AND source_snapshot_digest IS NOT NULL
         AND dependency_lock_digest IS NOT NULL
         AND build_profile_version IS NOT NULL
         AND build_generation IS NOT NULL)
        OR
        (source_kind = 'runtime_only'
         AND project_id IS NULL
         AND source_snapshot_digest IS NULL
         AND dependency_lock_digest IS NULL
         AND build_profile_version IS NULL
         AND build_generation IS NULL)
    ),
    CHECK (origin_kind <> 'build' OR source_kind = 'managed'),
    CHECK (source_snapshot_digest IS NULL OR (
        length(source_snapshot_digest) = 64
        AND lower(source_snapshot_digest) = source_snapshot_digest
        AND source_snapshot_digest NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (dependency_lock_digest IS NULL OR (
        length(dependency_lock_digest) = 64
        AND lower(dependency_lock_digest) = dependency_lock_digest
        AND dependency_lock_digest NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (build_profile_version IS NULL OR (
        length(build_profile_version) BETWEEN 1 AND 64
        AND build_profile_version NOT GLOB '*[^!-~]*'
    ))
);

INSERT INTO miniapp_releases_v079 (
    id, release_id, miniapp_id, owner_user_id, artifact_id,
    artifact_digest, manifest_digest, release_digest, origin_kind,
    origin_operation_id, source_kind, project_id, source_snapshot_digest,
    dependency_lock_digest, build_profile_version, build_generation,
    release_record_json, created_at
)
SELECT
    id, release_id, miniapp_id, owner_user_id, artifact_id,
    artifact_digest, manifest_digest, release_digest, origin_kind,
    origin_operation_id, source_kind, project_id, source_snapshot_digest,
    dependency_lock_digest, build_profile_version, build_generation,
    release_record_json, created_at
FROM miniapp_releases;

DROP TABLE miniapp_releases;
ALTER TABLE miniapp_releases_v079 RENAME TO miniapp_releases;

DELETE FROM sqlite_sequence
WHERE name = 'miniapp_releases';

INSERT INTO sqlite_sequence (name, seq)
SELECT
    'miniapp_releases',
    MAX(
        old_sequence,
        COALESCE((SELECT MAX(id) FROM miniapp_releases), 0)
    )
FROM miniapp_releases_v079_sequence;

DROP TABLE miniapp_releases_v079_sequence;

CREATE INDEX idx_miniapp_releases_artifact_id
    ON miniapp_releases(artifact_id);
CREATE INDEX idx_miniapp_releases_product
    ON miniapp_releases(owner_user_id, miniapp_id, created_at DESC);
CREATE INDEX idx_miniapp_releases_owner_user_id
    ON miniapp_releases(owner_user_id);
CREATE INDEX idx_miniapp_releases_miniapp_id
    ON miniapp_releases(miniapp_id);
CREATE INDEX idx_miniapp_releases_project_id
    ON miniapp_releases(project_id);
CREATE INDEX idx_miniapp_releases_origin_operation_id
    ON miniapp_releases(origin_operation_id);
CREATE INDEX idx_miniapp_releases_release_digest
    ON miniapp_releases(owner_user_id, miniapp_id, release_digest);
