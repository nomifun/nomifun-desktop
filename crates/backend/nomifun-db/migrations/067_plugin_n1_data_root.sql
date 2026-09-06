-- Plugin N1 clean data root.
--
-- This migration intentionally does not inspect, import, or alias the retired
-- Extension store. Relationships follow the v3 logical-reference contract:
-- canonical TEXT identities plus indexes and guard triggers, never physical
-- SQLite foreign keys.

CREATE TABLE plugin_artifacts (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    artifact_id              TEXT NOT NULL UNIQUE CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest          TEXT NOT NULL UNIQUE CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    package_id               TEXT NOT NULL CHECK (
        length(package_id) BETWEEN 1 AND 255
        AND package_id NOT GLOB '*[^A-Za-z0-9._-]*'
    ),
    package_version          TEXT NOT NULL CHECK (
        length(package_version) BETWEEN 1 AND 128
        AND package_version NOT GLOB '*[^!-~]*'
    ),
    manifest_digest          TEXT NOT NULL CHECK (
        length(manifest_digest) = 64
        AND lower(manifest_digest) = manifest_digest
        AND manifest_digest NOT GLOB '*[^0-9a-f]*'
    ),
    manifest_json            TEXT NOT NULL CHECK (
        json_valid(manifest_json)
        AND json_type(manifest_json) = 'object'
    ),
    managed_path             TEXT NOT NULL UNIQUE CHECK (
        managed_path <> ''
        AND substr(managed_path, 1, 1) <> '/'
        AND substr(managed_path, -1, 1) <> '/'
        AND instr(managed_path, '\') = 0
        AND instr(managed_path, '//') = 0
        AND instr('/' || managed_path || '/', '/../') = 0
        AND instr('/' || managed_path || '/', '/./') = 0
        AND instr(managed_path, char(0)) = 0
    ),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0)
);

CREATE TABLE plugin_projects (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id               TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id            TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    package_id               TEXT NOT NULL CHECK (
        length(package_id) BETWEEN 1 AND 255
        AND package_id NOT GLOB '*[^A-Za-z0-9._-]*'
    ),
    managed_source_path      TEXT CHECK (
        managed_source_path IS NULL OR (
            managed_source_path <> ''
            AND substr(managed_source_path, 1, 1) <> '/'
            AND substr(managed_source_path, -1, 1) <> '/'
            AND instr(managed_source_path, '\') = 0
            AND instr(managed_source_path, '//') = 0
            AND instr('/' || managed_source_path || '/', '/../') = 0
            AND instr('/' || managed_source_path || '/', '/./') = 0
            AND instr(managed_source_path, char(0)) = 0
        )
    ),
    source_head_digest       TEXT CHECK (
        source_head_digest IS NULL OR (
            length(source_head_digest) = 64
            AND lower(source_head_digest) = source_head_digest
            AND source_head_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    dependency_lock_digest   TEXT CHECK (
        dependency_lock_digest IS NULL OR (
            length(dependency_lock_digest) = 64
            AND lower(dependency_lock_digest) = dependency_lock_digest
            AND dependency_lock_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    build_generation         INTEGER NOT NULL DEFAULT 0 CHECK (build_generation >= 0),
    linked_mount_id          TEXT UNIQUE CHECK (
        linked_mount_id IS NULL OR (
            length(linked_mount_id) = 36
            AND lower(linked_mount_id) = linked_mount_id
            AND linked_mount_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(linked_mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    ready_candidate_id       TEXT UNIQUE CHECK (
        ready_candidate_id IS NULL OR (
            length(ready_candidate_id) = 36
            AND lower(ready_candidate_id) = ready_candidate_id
            AND ready_candidate_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(ready_candidate_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (
        managed_source_path IS NOT NULL
        OR (source_head_digest IS NULL AND dependency_lock_digest IS NULL)
    ),
    CHECK (
        source_head_digest IS NOT NULL
        OR dependency_lock_digest IS NULL
    )
);

CREATE TABLE product_operations (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id             TEXT NOT NULL UNIQUE CHECK (
        length(operation_id) = 36
        AND lower(operation_id) = operation_id
        AND operation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    kind                     TEXT NOT NULL CHECK (
        kind IN ('build', 'import', 'export', 'miniapp_permanent_delete')
    ),
    owner_kind               TEXT NOT NULL CHECK (
        owner_kind IN ('plugin_project', 'plugin_mount', 'miniapp')
    ),
    owner_id                 TEXT NOT NULL CHECK (
        length(owner_id) = 36
        AND lower(owner_id) = owner_id
        AND owner_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    state                    TEXT NOT NULL CHECK (
        state IN ('running', 'succeeded', 'failed', 'canceled')
    ),
    progress_percent         INTEGER CHECK (progress_percent BETWEEN 0 AND 100),
    last_error_code          TEXT CHECK (
        last_error_code IS NULL OR (
            length(last_error_code) BETWEEN 1 AND 256
            AND last_error_code NOT GLOB '*[^!-~]*'
        )
    ),
    bounded_log_tail_json    TEXT NOT NULL DEFAULT '[]' CHECK (
        json_valid(bounded_log_tail_json)
        AND json_type(bounded_log_tail_json) = 'array'
        AND json_array_length(bounded_log_tail_json) <= 200
    ),
    started_at_ms            INTEGER NOT NULL CHECK (started_at_ms > 0),
    finished_at_ms           INTEGER CHECK (
        finished_at_ms IS NULL OR finished_at_ms >= started_at_ms
    ),
    CHECK (
        (kind = 'build' AND owner_kind IN ('plugin_project', 'miniapp'))
        OR
        (kind IN ('import', 'export')
            AND owner_kind IN ('plugin_project', 'plugin_mount', 'miniapp'))
        OR
        (kind = 'miniapp_permanent_delete' AND owner_kind = 'miniapp')
    ),
    CHECK (
        kind <> 'miniapp_permanent_delete'
        OR progress_percent IS NULL
    ),
    CHECK (
        (state = 'running'
            AND finished_at_ms IS NULL
            AND last_error_code IS NULL)
        OR
        (state = 'succeeded'
            AND finished_at_ms IS NOT NULL
            AND last_error_code IS NULL
            AND (
                kind = 'miniapp_permanent_delete'
                OR progress_percent = 100
            ))
        OR
        (state = 'failed'
            AND finished_at_ms IS NOT NULL
            AND last_error_code IS NOT NULL)
        OR
        (state = 'canceled'
            AND kind <> 'miniapp_permanent_delete'
            AND finished_at_ms IS NOT NULL
            AND last_error_code IS NULL)
    )
);

CREATE TABLE plugin_ready_candidates (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    candidate_id             TEXT NOT NULL UNIQUE CHECK (
        length(candidate_id) = 36
        AND lower(candidate_id) = candidate_id
        AND candidate_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(candidate_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    project_id               TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    candidate_digest         TEXT NOT NULL UNIQUE CHECK (
        length(candidate_digest) = 64
        AND lower(candidate_digest) = candidate_digest
        AND candidate_digest NOT GLOB '*[^0-9a-f]*'
    ),
    origin_kind              TEXT NOT NULL CHECK (origin_kind IN ('build', 'import')),
    artifact_id              TEXT NOT NULL CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest          TEXT NOT NULL CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    base_target_digest       TEXT CHECK (
        base_target_digest IS NULL OR (
            length(base_target_digest) = 64
            AND lower(base_target_digest) = base_target_digest
            AND base_target_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    source_snapshot_digest   TEXT CHECK (
        source_snapshot_digest IS NULL OR (
            length(source_snapshot_digest) = 64
            AND lower(source_snapshot_digest) = source_snapshot_digest
            AND source_snapshot_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    dependency_lock_digest   TEXT CHECK (
        dependency_lock_digest IS NULL OR (
            length(dependency_lock_digest) = 64
            AND lower(dependency_lock_digest) = dependency_lock_digest
            AND dependency_lock_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    contract_diff_json       TEXT NOT NULL CHECK (
        json_valid(contract_diff_json)
        AND json_type(contract_diff_json) = 'object'
    ),
    origin_operation_id      TEXT NOT NULL CHECK (
        length(origin_operation_id) = 36
        AND lower(origin_operation_id) = origin_operation_id
        AND origin_operation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(origin_operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    build_generation         INTEGER NOT NULL CHECK (build_generation >= 0),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    CHECK (
        (source_snapshot_digest IS NULL AND dependency_lock_digest IS NULL)
        OR (source_snapshot_digest IS NOT NULL AND dependency_lock_digest IS NOT NULL)
    )
);

CREATE TABLE plugin_candidate_test_receipts (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    receipt_id               TEXT NOT NULL UNIQUE CHECK (
        length(receipt_id) = 36
        AND lower(receipt_id) = receipt_id
        AND receipt_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(receipt_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    candidate_id             TEXT NOT NULL UNIQUE CHECK (
        length(candidate_id) = 36
        AND lower(candidate_id) = candidate_id
        AND candidate_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(candidate_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    candidate_digest         TEXT NOT NULL CHECK (
        length(candidate_digest) = 64
        AND lower(candidate_digest) = candidate_digest
        AND candidate_digest NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_id              TEXT NOT NULL CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest          TEXT NOT NULL CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    receipt_digest           TEXT NOT NULL UNIQUE CHECK (
        length(receipt_digest) = 64
        AND lower(receipt_digest) = receipt_digest
        AND receipt_digest NOT GLOB '*[^0-9a-f]*'
    ),
    runtime_fingerprint_digest TEXT NOT NULL CHECK (
        length(runtime_fingerprint_digest) = 64
        AND lower(runtime_fingerprint_digest) = runtime_fingerprint_digest
        AND runtime_fingerprint_digest NOT GLOB '*[^0-9a-f]*'
    ),
    receipt_json             TEXT NOT NULL CHECK (
        json_valid(receipt_json)
        AND json_type(receipt_json) = 'object'
    ),
    tested_at                INTEGER NOT NULL CHECK (tested_at >= 0)
);

CREATE TABLE plugin_mounts (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    mount_id                 TEXT NOT NULL UNIQUE CHECK (
        length(mount_id) = 36
        AND lower(mount_id) = mount_id
        AND mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    package_id               TEXT NOT NULL UNIQUE CHECK (
        length(package_id) BETWEEN 1 AND 255
        AND package_id NOT GLOB '*[^A-Za-z0-9._-]*'
    ),
    current_artifact_digest  TEXT CHECK (
        current_artifact_digest IS NULL OR (
            length(current_artifact_digest) = 64
            AND lower(current_artifact_digest) = current_artifact_digest
            AND current_artifact_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    previous_artifact_digest TEXT CHECK (
        previous_artifact_digest IS NULL OR (
            length(previous_artifact_digest) = 64
            AND lower(previous_artifact_digest) = previous_artifact_digest
            AND previous_artifact_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    current_revision_id      TEXT CHECK (
        current_revision_id IS NULL OR (
            length(current_revision_id) = 36
            AND lower(current_revision_id) = current_revision_id
            AND current_revision_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(current_revision_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    previous_revision_id     TEXT CHECK (
        previous_revision_id IS NULL OR (
            length(previous_revision_id) = 36
            AND lower(previous_revision_id) = previous_revision_id
            AND previous_revision_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(previous_revision_id, '-', '') NOT GLOB '*[^0-9a-f]*'
        )
    ),
    enabled                  INTEGER NOT NULL DEFAULT 0 CHECK (
        typeof(enabled) = 'integer' AND enabled IN (0, 1)
    ),
    retained                 INTEGER NOT NULL DEFAULT 1 CHECK (
        typeof(retained) = 'integer' AND retained IN (0, 1)
    ),
    delete_pending           INTEGER NOT NULL DEFAULT 0 CHECK (
        typeof(delete_pending) = 'integer' AND delete_pending IN (0, 1)
    ),
    revision                 INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    config_json              TEXT NOT NULL DEFAULT '{}' CHECK (
        json_valid(config_json)
        AND json_type(config_json) = 'object'
    ),
    data_dir_path            TEXT NOT NULL UNIQUE CHECK (
        data_dir_path <> ''
        AND substr(data_dir_path, 1, 1) <> '/'
        AND substr(data_dir_path, -1, 1) <> '/'
        AND instr(data_dir_path, '\') = 0
        AND instr(data_dir_path, '//') = 0
        AND instr('/' || data_dir_path || '/', '/../') = 0
        AND instr('/' || data_dir_path || '/', '/./') = 0
        AND instr(data_dir_path, char(0)) = 0
    ),
    last_error               TEXT CHECK (
        last_error IS NULL OR length(last_error) BETWEEN 1 AND 8192
    ),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (
        (current_artifact_digest IS NULL) = (current_revision_id IS NULL)
    ),
    CHECK (
        (previous_artifact_digest IS NULL) = (previous_revision_id IS NULL)
    ),
    CHECK (
        current_revision_id IS NULL
        OR previous_revision_id IS NULL
        OR current_revision_id <> previous_revision_id
    ),
    CHECK (
        current_artifact_digest IS NULL
        OR previous_artifact_digest IS NULL
        OR current_artifact_digest <> previous_artifact_digest
    ),
    CHECK (
        enabled = 0
        OR (
            current_artifact_digest IS NOT NULL
            AND retained = 0
            AND delete_pending = 0
        )
    ),
    CHECK (
        retained = 0
        OR (
            enabled = 0
            AND current_artifact_digest IS NULL
            AND previous_artifact_digest IS NULL
            AND current_revision_id IS NULL
            AND previous_revision_id IS NULL
        )
    ),
    CHECK (
        delete_pending = 0
        OR retained = 1
    ),
    CHECK (
        retained = 1
        OR current_artifact_digest IS NOT NULL
    )
);

CREATE TABLE plugin_mount_revisions (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    mount_revision_id        TEXT NOT NULL UNIQUE CHECK (
        length(mount_revision_id) = 36
        AND lower(mount_revision_id) = mount_revision_id
        AND mount_revision_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_revision_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    mount_id                 TEXT NOT NULL CHECK (
        length(mount_id) = 36
        AND lower(mount_id) = mount_id
        AND mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    revision                 INTEGER NOT NULL CHECK (revision >= 1),
    artifact_id              TEXT NOT NULL CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest          TEXT NOT NULL CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    candidate_key            TEXT NOT NULL UNIQUE CHECK (
        length(candidate_key) = 36
        AND lower(candidate_key) = candidate_key
        AND candidate_key GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(candidate_key, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    candidate_digest         TEXT NOT NULL CHECK (
        length(candidate_digest) = 64
        AND lower(candidate_digest) = candidate_digest
        AND candidate_digest NOT GLOB '*[^0-9a-f]*'
    ),
    base_target_digest       TEXT CHECK (
        base_target_digest IS NULL OR (
            length(base_target_digest) = 64
            AND lower(base_target_digest) = base_target_digest
            AND base_target_digest NOT GLOB '*[^0-9a-f]*'
        )
    ),
    applied_at               INTEGER NOT NULL CHECK (applied_at >= 0),
    UNIQUE (mount_id, revision)
);

CREATE TABLE plugin_credential_bindings (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    mount_id                 TEXT NOT NULL CHECK (
        length(mount_id) = 36
        AND lower(mount_id) = mount_id
        AND mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    slot                     TEXT NOT NULL CHECK (
        length(slot) BETWEEN 1 AND 128
        AND slot NOT GLOB '*[^!-~]*'
    ),
    credential_id            TEXT NOT NULL CHECK (
        length(credential_id) BETWEEN 1 AND 512
        AND credential_id NOT GLOB '*[^!-~]*'
    ),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (mount_id, slot)
);

CREATE TABLE plugin_kv (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    mount_id                 TEXT NOT NULL CHECK (
        length(mount_id) = 36
        AND lower(mount_id) = mount_id
        AND mount_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(mount_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    namespace                TEXT NOT NULL CHECK (
        length(namespace) BETWEEN 1 AND 128
        AND namespace NOT GLOB '*[^!-~]*'
    ),
    key                      TEXT NOT NULL CHECK (
        length(key) BETWEEN 1 AND 256
        AND key NOT GLOB '*[^!-~]*'
    ),
    value_json               TEXT NOT NULL CHECK (json_valid(value_json)),
    revision                 INTEGER NOT NULL CHECK (revision >= 1),
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (mount_id, namespace, key)
);

CREATE INDEX idx_plugin_artifacts_package_id
    ON plugin_artifacts(package_id, package_version, artifact_id);
CREATE INDEX idx_plugin_projects_owner_user_id
    ON plugin_projects(owner_user_id, project_id);
CREATE INDEX idx_plugin_projects_linked_mount_id
    ON plugin_projects(linked_mount_id);
CREATE INDEX idx_plugin_projects_ready_candidate_id
    ON plugin_projects(ready_candidate_id);
CREATE INDEX idx_product_operations_plugin_project_owner_id
    ON product_operations(owner_id, started_at_ms, operation_id)
    WHERE owner_kind = 'plugin_project';
CREATE INDEX idx_product_operations_plugin_mount_owner_id
    ON product_operations(owner_id, started_at_ms, operation_id)
    WHERE owner_kind = 'plugin_mount';
CREATE INDEX idx_product_operations_miniapp_owner_id
    ON product_operations(owner_id, started_at_ms, operation_id)
    WHERE owner_kind = 'miniapp';
CREATE INDEX idx_plugin_ready_candidates_project_id
    ON plugin_ready_candidates(project_id);
CREATE INDEX idx_plugin_ready_candidates_artifact_id
    ON plugin_ready_candidates(artifact_id);
CREATE INDEX idx_plugin_ready_candidates_artifact_digest
    ON plugin_ready_candidates(artifact_digest);
CREATE INDEX idx_plugin_ready_candidates_origin_operation_id
    ON plugin_ready_candidates(origin_operation_id);
CREATE INDEX idx_plugin_candidate_test_receipts_candidate_id
    ON plugin_candidate_test_receipts(candidate_id);
CREATE INDEX idx_plugin_candidate_test_receipts_artifact_id
    ON plugin_candidate_test_receipts(artifact_id);
CREATE INDEX idx_plugin_candidate_test_receipts_artifact_digest
    ON plugin_candidate_test_receipts(artifact_digest);
CREATE INDEX idx_plugin_mounts_current_revision_id
    ON plugin_mounts(current_revision_id);
CREATE INDEX idx_plugin_mounts_previous_revision_id
    ON plugin_mounts(previous_revision_id);
CREATE INDEX idx_plugin_mount_revisions_mount_id
    ON plugin_mount_revisions(mount_id, revision);
CREATE INDEX idx_plugin_mount_revisions_artifact_id
    ON plugin_mount_revisions(artifact_id);
CREATE INDEX idx_plugin_mount_revisions_artifact_digest
    ON plugin_mount_revisions(artifact_digest);
CREATE INDEX idx_plugin_credential_bindings_mount_id
    ON plugin_credential_bindings(mount_id, slot);
CREATE INDEX idx_plugin_credential_bindings_credential_id
    ON plugin_credential_bindings(credential_id);
CREATE INDEX idx_plugin_kv_mount_id
    ON plugin_kv(mount_id, namespace, key);

CREATE TRIGGER trg_plugin_artifacts_immutable
BEFORE UPDATE ON plugin_artifacts
BEGIN
    SELECT RAISE(ABORT, 'plugin artifacts are immutable');
END;

CREATE TRIGGER trg_plugin_candidate_receipts_immutable
BEFORE UPDATE ON plugin_candidate_test_receipts
BEGIN
    SELECT RAISE(ABORT, 'plugin candidate test receipts are immutable');
END;

CREATE TRIGGER trg_plugin_candidate_receipts_exact_insert
BEFORE INSERT ON plugin_candidate_test_receipts
WHEN NOT EXISTS (
    SELECT 1
    FROM plugin_ready_candidates candidate
    WHERE candidate.candidate_id = NEW.candidate_id
      AND candidate.candidate_digest = NEW.candidate_digest
      AND candidate.artifact_id = NEW.artifact_id
      AND candidate.artifact_digest = NEW.artifact_digest
)
BEGIN
    SELECT RAISE(ABORT, 'plugin candidate test receipt must bind one exact candidate');
END;

CREATE TRIGGER trg_plugin_mount_revisions_immutable
BEFORE UPDATE ON plugin_mount_revisions
BEGIN
    SELECT RAISE(ABORT, 'plugin mount revisions are immutable');
END;

CREATE TRIGGER trg_plugin_mount_revision_insert_guard
BEFORE INSERT ON plugin_mount_revisions
WHEN NOT EXISTS (
        SELECT 1
        FROM plugin_mounts mount
        JOIN plugin_ready_candidates candidate
          ON candidate.candidate_id = NEW.candidate_key
        JOIN plugin_artifacts artifact
          ON artifact.artifact_id = NEW.artifact_id
         AND artifact.artifact_digest = NEW.artifact_digest
        WHERE mount.mount_id = NEW.mount_id
          AND candidate.candidate_digest = NEW.candidate_digest
          AND candidate.artifact_id = NEW.artifact_id
          AND candidate.artifact_digest = NEW.artifact_digest
          AND candidate.base_target_digest IS mount.current_artifact_digest
          AND artifact.package_id = mount.package_id
          AND NEW.revision = mount.revision + 1
    )
BEGIN
    SELECT RAISE(ABORT, 'plugin mount revision requires exact candidate, base, artifact, and next revision');
END;

CREATE TRIGGER trg_plugin_mount_pointer_insert_guard
BEFORE INSERT ON plugin_mounts
WHEN (
    NEW.current_revision_id IS NOT NULL
    OR NEW.previous_revision_id IS NOT NULL
)
BEGIN
    SELECT RAISE(ABORT, 'plugin mount must be created without executable pointers');
END;

CREATE TRIGGER trg_plugin_mount_pointer_update_guard
BEFORE UPDATE OF current_artifact_digest, previous_artifact_digest,
                 current_revision_id, previous_revision_id
ON plugin_mounts
WHEN (
    (
        NEW.current_revision_id IS NOT NULL
        AND NOT EXISTS (
            SELECT 1
            FROM plugin_mount_revisions revision
            WHERE revision.mount_revision_id = NEW.current_revision_id
              AND revision.mount_id = NEW.mount_id
              AND revision.artifact_digest = NEW.current_artifact_digest
        )
    )
    OR
    (
        NEW.previous_revision_id IS NOT NULL
        AND NOT EXISTS (
            SELECT 1
            FROM plugin_mount_revisions revision
            WHERE revision.mount_revision_id = NEW.previous_revision_id
              AND revision.mount_id = NEW.mount_id
              AND revision.artifact_digest = NEW.previous_artifact_digest
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin mount executable pointers require exact mount revisions');
END;

CREATE TRIGGER trg_plugin_mount_transition_shape_guard
BEFORE UPDATE OF current_artifact_digest, previous_artifact_digest,
                 current_revision_id, previous_revision_id, revision
ON plugin_mounts
WHEN (
    NEW.current_artifact_digest IS NOT OLD.current_artifact_digest
    OR NEW.previous_artifact_digest IS NOT OLD.previous_artifact_digest
    OR NEW.current_revision_id IS NOT OLD.current_revision_id
    OR NEW.previous_revision_id IS NOT OLD.previous_revision_id
)
AND (
    NEW.revision <> OLD.revision + 1
    OR NOT (
        (
            NEW.current_revision_id IS NULL
            AND NEW.previous_revision_id IS NULL
            AND NEW.current_artifact_digest IS NULL
            AND NEW.previous_artifact_digest IS NULL
            AND NEW.enabled = 0
            AND NEW.retained = 1
        )
        OR
        (
            OLD.previous_revision_id IS NOT NULL
            AND NEW.current_revision_id IS OLD.previous_revision_id
            AND NEW.previous_revision_id IS OLD.current_revision_id
            AND NEW.current_artifact_digest IS OLD.previous_artifact_digest
            AND NEW.previous_artifact_digest IS OLD.current_artifact_digest
        )
        OR
        (
            NEW.previous_revision_id IS OLD.current_revision_id
            AND NEW.previous_artifact_digest IS OLD.current_artifact_digest
            AND EXISTS (
                SELECT 1
                FROM plugin_mount_revisions revision
                WHERE revision.mount_revision_id = NEW.current_revision_id
                  AND revision.mount_id = NEW.mount_id
                  AND revision.revision = NEW.revision
                  AND revision.artifact_digest = NEW.current_artifact_digest
            )
        )
    )
)
BEGIN
    SELECT RAISE(ABORT, 'plugin mount transition must be exact apply, restore, or uninstall');
END;

CREATE TRIGGER trg_plugin_project_initial_pointer_guard
BEFORE INSERT ON plugin_projects
WHEN NEW.linked_mount_id IS NOT NULL OR NEW.ready_candidate_id IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'plugin project must be created without linked or ready pointers');
END;

CREATE TRIGGER trg_plugin_project_ready_candidate_update_guard
BEFORE UPDATE OF ready_candidate_id ON plugin_projects
WHEN NEW.ready_candidate_id IS NOT NULL
 AND NOT EXISTS (
    SELECT 1
    FROM plugin_ready_candidates candidate
    WHERE candidate.candidate_id = NEW.ready_candidate_id
      AND candidate.project_id = NEW.project_id
      AND candidate.build_generation = NEW.build_generation
)
BEGIN
    SELECT RAISE(ABORT, 'plugin project ready pointer requires its exact current-generation candidate');
END;

CREATE TRIGGER trg_plugin_ready_candidate_insert_guard
BEFORE INSERT ON plugin_ready_candidates
WHEN NOT EXISTS (
        SELECT 1
        FROM plugin_projects project
        JOIN plugin_artifacts artifact
          ON artifact.artifact_id = NEW.artifact_id
         AND artifact.artifact_digest = NEW.artifact_digest
        JOIN product_operations operation
          ON operation.operation_id = NEW.origin_operation_id
        WHERE project.project_id = NEW.project_id
          AND project.package_id = artifact.package_id
          AND project.build_generation = NEW.build_generation
          AND operation.kind = NEW.origin_kind
          AND operation.owner_kind = 'plugin_project'
          AND operation.owner_id = project.project_id
          AND operation.kind IN ('build', 'import')
          AND operation.state = 'succeeded'
          AND (
              (
                  NEW.origin_kind = 'import'
                  AND project.managed_source_path IS NULL
                  AND NEW.source_snapshot_digest IS NULL
                  AND NEW.dependency_lock_digest IS NULL
              )
              OR
              (
                  project.managed_source_path IS NOT NULL
                  AND NEW.build_generation > 0
                  AND NEW.source_snapshot_digest = project.source_head_digest
                  AND NEW.dependency_lock_digest = project.dependency_lock_digest
              )
          )
    )
BEGIN
    SELECT RAISE(ABORT, 'plugin ready candidate requires exact project generation, artifact, and successful origin operation');
END;

CREATE TRIGGER trg_product_operations_terminal_immutable
BEFORE UPDATE ON product_operations
WHEN OLD.state <> 'running'
BEGIN
    SELECT RAISE(ABORT, 'terminal product operations are immutable');
END;

CREATE TRIGGER trg_product_operation_log_insert_guard
BEFORE INSERT ON product_operations
WHEN EXISTS (
    SELECT 1
    FROM json_each(NEW.bounded_log_tail_json) entry
    WHERE entry.type <> 'text'
       OR length(entry.value) > 4096
       OR instr(entry.value, char(0)) > 0
)
BEGIN
    SELECT RAISE(ABORT, 'product operation log tail lines must be bounded strings');
END;

CREATE TRIGGER trg_product_operation_log_update_guard
BEFORE UPDATE OF bounded_log_tail_json ON product_operations
WHEN EXISTS (
    SELECT 1
    FROM json_each(NEW.bounded_log_tail_json) entry
    WHERE entry.type <> 'text'
       OR length(entry.value) > 4096
       OR instr(entry.value, char(0)) > 0
)
BEGIN
    SELECT RAISE(ABORT, 'product operation log tail lines must be bounded strings');
END;
