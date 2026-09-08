-- Immutable lineage captured when a MiniApp Build Operation starts.
--
-- This table is deliberately separate from the shared Plugin operation rows:
-- MiniApp Build success is only valid through the MiniApp repository's
-- atomic Artifact/Release/Ready commit. No foreign keys or triggers are used;
-- the repository owns the logical reference checks.

CREATE TABLE miniapp_build_operation_lineage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id TEXT NOT NULL UNIQUE CHECK (
        length(operation_id) = 36
        AND lower(operation_id) = operation_id
        AND operation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    miniapp_id TEXT NOT NULL CHECK (
        length(miniapp_id) = 36
        AND lower(miniapp_id) = miniapp_id
        AND miniapp_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(miniapp_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    project_id TEXT NOT NULL CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    project_revision INTEGER NOT NULL CHECK (project_revision >= 1),
    source_snapshot_digest TEXT NOT NULL CHECK (
        length(source_snapshot_digest) = 64
        AND lower(source_snapshot_digest) = source_snapshot_digest
        AND source_snapshot_digest NOT GLOB '*[^0-9a-f]*'
    ),
    dependency_lock_digest TEXT NOT NULL CHECK (
        length(dependency_lock_digest) = 64
        AND lower(dependency_lock_digest) = dependency_lock_digest
        AND dependency_lock_digest NOT GLOB '*[^0-9a-f]*'
    ),
    build_profile_version TEXT NOT NULL CHECK (
        length(build_profile_version) BETWEEN 1 AND 64
        AND build_profile_version NOT GLOB '*[^!-~]*'
    ),
    build_generation INTEGER NOT NULL CHECK (build_generation >= 1),
    started_at_ms INTEGER NOT NULL CHECK (started_at_ms > 0),
    UNIQUE (owner_user_id, operation_id, miniapp_id, project_id)
);

CREATE INDEX idx_miniapp_build_operation_lineage_owner
    ON miniapp_build_operation_lineage(owner_user_id, miniapp_id, started_at_ms DESC);
CREATE INDEX idx_miniapp_build_operation_lineage_miniapp_id
    ON miniapp_build_operation_lineage(miniapp_id);
CREATE INDEX idx_miniapp_build_operation_lineage_project
    ON miniapp_build_operation_lineage(project_id, owner_user_id, build_generation);
CREATE INDEX idx_miniapp_build_operation_lineage_operation_id
    ON miniapp_build_operation_lineage(operation_id);
