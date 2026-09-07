-- Phase M1-0 clean-start data root.
--
-- This migration is independent from the retired `miniapps` table. It does
-- not inspect, copy, alias, or mutate that table. Fresh-v4 keeps relational
-- ownership in the executable logical-reference registry; physical foreign
-- keys and triggers are intentionally forbidden by id_schema_contract.

CREATE TABLE miniapp_library_state (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    singleton_key TEXT NOT NULL CHECK (singleton_key = 'miniapp_m1'),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= 0),
    UNIQUE (singleton_key, owner_user_id)
);

CREATE TABLE miniapp_products (
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
    product_revision INTEGER NOT NULL DEFAULT 1 CHECK (product_revision >= 1),
    display_name TEXT NOT NULL CHECK (length(display_name) BETWEEN 1 AND 255),
    description TEXT,
    icon_asset_id TEXT,
    kind TEXT NOT NULL CHECK (kind IN ('ui_only', 'service')),
    lifecycle TEXT NOT NULL DEFAULT 'disabled'
        CHECK (lifecycle IN ('enabled', 'disabled', 'trashed', 'deleting')),
    pointer_revision INTEGER NOT NULL DEFAULT 1 CHECK (pointer_revision >= 1),
    active_release_epoch INTEGER NOT NULL DEFAULT 0
        CHECK (active_release_epoch >= 0),
    ready_release_id TEXT,
    ready_release_digest TEXT,
    active_release_id TEXT,
    active_release_digest TEXT,
    previous_release_id TEXT,
    previous_release_digest TEXT,
    materialized_catalog_digest TEXT NOT NULL CHECK (
        length(materialized_catalog_digest) = 64
        AND lower(materialized_catalog_digest) = materialized_catalog_digest
        AND materialized_catalog_digest NOT GLOB '*[^0-9a-f]*'
    ),
    config_schema_json TEXT NOT NULL DEFAULT '{"type":"object"}'
        CHECK (json_valid(config_schema_json) AND json_type(config_schema_json) = 'object'),
    config_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(config_json) AND json_type(config_json) = 'object'),
    config_revision INTEGER NOT NULL DEFAULT 1 CHECK (config_revision >= 1),
    credential_bindings_revision INTEGER NOT NULL DEFAULT 1
        CHECK (credential_bindings_revision >= 1),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (owner_user_id, miniapp_id),
    CHECK ((ready_release_id IS NULL) = (ready_release_digest IS NULL)),
    CHECK ((active_release_id IS NULL) = (active_release_digest IS NULL)),
    CHECK ((previous_release_id IS NULL) = (previous_release_digest IS NULL)),
    CHECK (ready_release_digest IS NULL OR (
        length(ready_release_digest) = 64
        AND lower(ready_release_digest) = ready_release_digest
        AND ready_release_digest NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (active_release_digest IS NULL OR (
        length(active_release_digest) = 64
        AND lower(active_release_digest) = active_release_digest
        AND active_release_digest NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (previous_release_digest IS NULL OR (
        length(previous_release_digest) = 64
        AND lower(previous_release_digest) = previous_release_digest
        AND previous_release_digest NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (ready_release_id IS NULL OR (
        length(ready_release_id) = 36
        AND lower(ready_release_id) = ready_release_id
        AND ready_release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(ready_release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (icon_asset_id IS NULL OR (
        length(icon_asset_id) = 36
        AND lower(icon_asset_id) = icon_asset_id
        AND icon_asset_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(icon_asset_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (active_release_id IS NULL OR (
        length(active_release_id) = 36
        AND lower(active_release_id) = active_release_id
        AND active_release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(active_release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK (previous_release_id IS NULL OR (
        length(previous_release_id) = 36
        AND lower(previous_release_id) = previous_release_id
        AND previous_release_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(previous_release_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )),
    CHECK ((active_release_id IS NULL) = (active_release_epoch = 0)),
    CHECK (active_release_id IS NULL OR (
        active_release_id <> ready_release_id
        AND active_release_id <> previous_release_id
    )),
    CHECK (previous_release_id IS NULL OR previous_release_id <> ready_release_id)
);

CREATE TABLE miniapp_projects (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    project_id TEXT NOT NULL UNIQUE CHECK (
        length(project_id) = 36
        AND lower(project_id) = project_id
        AND project_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(project_id, '-', '') NOT GLOB '*[^0-9a-f]*'
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
    project_revision INTEGER NOT NULL DEFAULT 1 CHECK (project_revision >= 1),
    source_state TEXT NOT NULL DEFAULT 'empty'
        CHECK (source_state IN ('empty', 'editable', 'runtime_only')),
    managed_source_path TEXT,
    source_head_digest TEXT,
    dependency_lock_digest TEXT,
    build_profile_version TEXT,
    build_generation INTEGER NOT NULL DEFAULT 0 CHECK (build_generation >= 0),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (owner_user_id, project_id),
    UNIQUE (owner_user_id, project_id, miniapp_id),
    CHECK (
        (source_state = 'empty'
         AND managed_source_path IS NULL
         AND source_head_digest IS NULL
         AND dependency_lock_digest IS NULL
         AND build_profile_version IS NULL
         AND build_generation = 0)
        OR
        (source_state = 'editable'
         AND managed_source_path IS NOT NULL
         AND source_head_digest IS NOT NULL
         AND dependency_lock_digest IS NOT NULL
         AND build_profile_version IS NOT NULL
         AND build_generation > 0)
        OR
        (source_state = 'runtime_only'
         AND managed_source_path IS NULL
         AND source_head_digest IS NULL
         AND dependency_lock_digest IS NULL
         AND build_profile_version IS NULL
         AND build_generation = 0)
    ),
    CHECK (managed_source_path IS NULL OR (
        managed_source_path <> ''
        AND substr(managed_source_path, 1, 1) <> '/'
        AND substr(managed_source_path, -1, 1) <> '/'
        AND instr(managed_source_path, '\') = 0
        AND instr(managed_source_path, '//') = 0
        AND instr('/' || managed_source_path || '/', '/../') = 0
        AND instr('/' || managed_source_path || '/', '/./') = 0
        AND instr(managed_source_path, char(0)) = 0
    )),
    CHECK (source_head_digest IS NULL OR (
        length(source_head_digest) = 64
        AND lower(source_head_digest) = source_head_digest
        AND source_head_digest NOT GLOB '*[^0-9a-f]*'
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

CREATE TABLE miniapp_release_artifacts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    artifact_id TEXT NOT NULL UNIQUE CHECK (
        length(artifact_id) = 36
        AND lower(artifact_id) = artifact_id
        AND artifact_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(artifact_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL CHECK (
        length(owner_user_id) = 36
        AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_digest TEXT NOT NULL UNIQUE CHECK (
        length(artifact_digest) = 64
        AND lower(artifact_digest) = artifact_digest
        AND artifact_digest NOT GLOB '*[^0-9a-f]*'
    ),
    manifest_digest TEXT NOT NULL CHECK (
        length(manifest_digest) = 64
        AND lower(manifest_digest) = manifest_digest
        AND manifest_digest NOT GLOB '*[^0-9a-f]*'
    ),
    artifact_record_json TEXT NOT NULL
        CHECK (json_valid(artifact_record_json)
               AND json_type(artifact_record_json) = 'object'),
    managed_path TEXT NOT NULL UNIQUE CHECK (
        managed_path <> ''
        AND substr(managed_path, 1, 1) <> '/'
        AND substr(managed_path, -1, 1) <> '/'
        AND instr(managed_path, '\') = 0
        AND instr(managed_path, '//') = 0
        AND instr('/' || managed_path || '/', '/../') = 0
        AND instr('/' || managed_path || '/', '/./') = 0
        AND instr(managed_path, char(0)) = 0
    ),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    UNIQUE (owner_user_id, artifact_id, artifact_digest, manifest_digest)
);

CREATE TABLE miniapp_releases (
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
    release_digest TEXT NOT NULL UNIQUE CHECK (
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
    UNIQUE (owner_user_id, release_digest),
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

CREATE TABLE miniapp_credential_bindings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
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
    slot_key TEXT NOT NULL CHECK (
        length(slot_key) BETWEEN 1 AND 128
        AND slot_key NOT GLOB '*[^!-~]*'
    ),
    credential_id TEXT NOT NULL CHECK (
        length(credential_id) BETWEEN 1 AND 512
        AND credential_id NOT GLOB '*[^!-~]*'
    ),
    created_at INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (owner_user_id, miniapp_id, slot_key)
);

CREATE INDEX idx_miniapp_library_state_owner
    ON miniapp_library_state(owner_user_id, revision);
CREATE INDEX idx_miniapp_library_state_owner_user_id
    ON miniapp_library_state(owner_user_id);
CREATE INDEX idx_miniapp_products_owner
    ON miniapp_products(owner_user_id, updated_at DESC, id DESC);
CREATE INDEX idx_miniapp_products_owner_user_id
    ON miniapp_products(owner_user_id);
CREATE INDEX idx_miniapp_products_icon_asset_id
    ON miniapp_products(icon_asset_id);
CREATE INDEX idx_miniapp_products_ready_release_id
    ON miniapp_products(ready_release_id);
CREATE INDEX idx_miniapp_products_active_release_id
    ON miniapp_products(active_release_id);
CREATE INDEX idx_miniapp_products_previous_release_id
    ON miniapp_products(previous_release_id);
CREATE INDEX idx_miniapp_projects_owner
    ON miniapp_projects(owner_user_id, miniapp_id, updated_at DESC);
CREATE INDEX idx_miniapp_projects_owner_user_id
    ON miniapp_projects(owner_user_id);
CREATE INDEX idx_miniapp_projects_miniapp_id
    ON miniapp_projects(miniapp_id);
CREATE INDEX idx_miniapp_release_artifacts_owner
    ON miniapp_release_artifacts(owner_user_id, artifact_digest);
CREATE INDEX idx_miniapp_release_artifacts_owner_user_id
    ON miniapp_release_artifacts(owner_user_id);
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
CREATE INDEX idx_miniapp_credential_bindings_product
    ON miniapp_credential_bindings(owner_user_id, miniapp_id, slot_key);
CREATE INDEX idx_miniapp_credential_bindings_owner_user_id
    ON miniapp_credential_bindings(owner_user_id);
CREATE INDEX idx_miniapp_credential_bindings_miniapp_id
    ON miniapp_credential_bindings(miniapp_id);
CREATE INDEX idx_miniapp_credential_bindings_credential_id
    ON miniapp_credential_bindings(credential_id);
