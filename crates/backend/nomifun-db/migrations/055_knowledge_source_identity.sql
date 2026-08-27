-- Normalized URL-source identity and provenance for filesystem-backed
-- knowledge entries.
--
-- The filesystem remains the content source of truth and `knowledge_entries`
-- remains a rebuildable identity/location projection. These tables own the
-- durable source aggregate, per-URL synchronization state, and the relationship
-- between a source item and a stable entry ID. Relative paths are deliberately
-- absent: a managed document may be renamed or moved without losing its source
-- identity.
--
-- Relationships follow the v3 logical-reference convention: canonical UUIDv7
-- TEXT columns plus indexes, repository-coordinated cleanup, and no physical
-- SQLite foreign keys.

CREATE TABLE knowledge_sources (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_source_id      TEXT NOT NULL UNIQUE
                             CHECK (
                                 length(knowledge_source_id) = 36
                                 AND lower(knowledge_source_id) = knowledge_source_id
                                 AND knowledge_source_id GLOB '????????-????-7???-[89ab]???-????????????'
                                 AND replace(knowledge_source_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                             ),
    knowledge_base_id        TEXT NOT NULL
                             CHECK (
                                 length(knowledge_base_id) = 36
                                 AND lower(knowledge_base_id) = knowledge_base_id
                                 AND knowledge_base_id GLOB '????????-????-7???-[89ab]???-????????????'
                                 AND replace(knowledge_base_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                             ),
    kind                     TEXT NOT NULL CHECK (kind = 'url'),
    mode                     TEXT NOT NULL CHECK (mode IN ('live', 'snapshot')),
    state                    TEXT NOT NULL CHECK (state IN ('active', 'paused', 'removed')),
    revision                 INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    default_parent_entry_id  TEXT
                             CHECK (
                                 default_parent_entry_id IS NULL
                                 OR (
                                     length(default_parent_entry_id) = 36
                                     AND lower(default_parent_entry_id) = default_parent_entry_id
                                     AND default_parent_entry_id GLOB '????????-????-7???-[89ab]???-????????????'
                                     AND replace(default_parent_entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                                 )
                             ),
    removed_at               INTEGER,
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (removed_at IS NULL OR removed_at >= created_at),
    CHECK (state <> 'removed' OR default_parent_entry_id IS NULL),
    CHECK (
        (state = 'removed' AND removed_at IS NOT NULL)
        OR (state <> 'removed' AND removed_at IS NULL)
    )
);

CREATE INDEX idx_knowledge_sources_knowledge_base_id
    ON knowledge_sources(knowledge_base_id, state, created_at, knowledge_source_id);

CREATE INDEX idx_knowledge_sources_default_parent_entry_id
    ON knowledge_sources(default_parent_entry_id)
    WHERE default_parent_entry_id IS NOT NULL;

CREATE UNIQUE INDEX uq_knowledge_sources_live_kind
    ON knowledge_sources(knowledge_base_id, kind)
    WHERE state <> 'removed';

CREATE TABLE knowledge_source_items (
    id                        INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_source_item_id  TEXT NOT NULL UNIQUE
                              CHECK (
                                  length(knowledge_source_item_id) = 36
                                  AND lower(knowledge_source_item_id) = knowledge_source_item_id
                                  AND knowledge_source_item_id GLOB '????????-????-7???-[89ab]???-????????????'
                                  AND replace(knowledge_source_item_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                              ),
    knowledge_source_id       TEXT NOT NULL
                              CHECK (
                                  length(knowledge_source_id) = 36
                                  AND lower(knowledge_source_id) = knowledge_source_id
                                  AND knowledge_source_id GLOB '????????-????-7???-[89ab]???-????????????'
                                  AND replace(knowledge_source_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                              ),
    requested_url             TEXT NOT NULL
                              CHECK (
                                  length(requested_url) BETWEEN 1 AND 8192
                                  AND trim(requested_url) = requested_url
                                  AND instr(requested_url, char(0)) = 0
                              ),
    normalized_url            TEXT NOT NULL
                              CHECK (
                                  length(normalized_url) BETWEEN 1 AND 8192
                                  AND trim(normalized_url) = normalized_url
                                  AND instr(normalized_url, char(0)) = 0
                              ),
    final_url                 TEXT
                              CHECK (
                                  final_url IS NULL
                                  OR (
                                      length(final_url) BETWEEN 1 AND 8192
                                      AND trim(final_url) = final_url
                                      AND instr(final_url, char(0)) = 0
                                  )
                              ),
    rendered                  INTEGER NOT NULL DEFAULT 0 CHECK (rendered IN (0, 1)),
    title                     TEXT
                              CHECK (
                                  title IS NULL
                                  OR (
                                      length(title) BETWEEN 1 AND 1024
                                      AND trim(title) = title
                                      AND instr(title, char(0)) = 0
                                  )
                              ),
    ordinal                   INTEGER NOT NULL CHECK (ordinal >= 0),
    state                     TEXT NOT NULL CHECK (state IN ('active', 'paused', 'removed')),
    sync_status               TEXT NOT NULL CHECK (sync_status IN (
                                  'pending',
                                  'syncing',
                                  'synced',
                                  'failed',
                                  'conflicted',
                                  'missing'
                              )),
    revision                  INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    etag                      TEXT
                              CHECK (
                                  etag IS NULL
                                  OR (length(etag) BETWEEN 1 AND 4096 AND instr(etag, char(0)) = 0)
                              ),
    http_last_modified        TEXT
                              CHECK (
                                  http_last_modified IS NULL
                                  OR (
                                      length(http_last_modified) BETWEEN 1 AND 512
                                      AND instr(http_last_modified, char(0)) = 0
                                  )
                              ),
    last_attempt_at           INTEGER CHECK (last_attempt_at IS NULL OR last_attempt_at >= 0),
    last_success_at           INTEGER CHECK (last_success_at IS NULL OR last_success_at >= 0),
    last_error                TEXT
                              CHECK (
                                  last_error IS NULL
                                  OR (length(last_error) BETWEEN 1 AND 8192 AND instr(last_error, char(0)) = 0)
                              ),
    last_published_hash       TEXT
                              CHECK (
                                  last_published_hash IS NULL
                                  OR (
                                      length(last_published_hash) = 64
                                      AND lower(last_published_hash) = last_published_hash
                                      AND last_published_hash NOT GLOB '*[^0-9a-f]*'
                                  )
                              ),
    removed_at                INTEGER,
    created_at                INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at                INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (removed_at IS NULL OR removed_at >= created_at),
    CHECK (state <> 'removed' OR sync_status <> 'syncing'),
    CHECK (sync_status <> 'syncing' OR last_attempt_at IS NOT NULL),
    CHECK (
        (state = 'removed' AND removed_at IS NOT NULL)
        OR (state <> 'removed' AND removed_at IS NULL)
    )
);

CREATE INDEX idx_knowledge_source_items_knowledge_source_id
    ON knowledge_source_items(knowledge_source_id, state, ordinal, knowledge_source_item_id);

CREATE UNIQUE INDEX uq_knowledge_source_items_live_normalized_url
    ON knowledge_source_items(knowledge_source_id, normalized_url)
    WHERE state <> 'removed';

CREATE UNIQUE INDEX uq_knowledge_source_items_live_ordinal
    ON knowledge_source_items(knowledge_source_id, ordinal)
    WHERE state <> 'removed';

CREATE TABLE knowledge_entry_provenance (
    id                        INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_entry_id        TEXT NOT NULL
                              CHECK (
                                  length(knowledge_entry_id) = 36
                                  AND lower(knowledge_entry_id) = knowledge_entry_id
                                  AND knowledge_entry_id GLOB '????????-????-7???-[89ab]???-????????????'
                                  AND replace(knowledge_entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                              ),
    knowledge_source_item_id  TEXT NOT NULL
                              CHECK (
                                  length(knowledge_source_item_id) = 36
                                  AND lower(knowledge_source_item_id) = knowledge_source_item_id
                                  AND knowledge_source_item_id GLOB '????????-????-7???-[89ab]???-????????????'
                                  AND replace(knowledge_source_item_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                              ),
    relationship              TEXT NOT NULL CHECK (relationship IN ('managed', 'detached', 'copy')),
    derived_from_entry_id     TEXT
                              CHECK (
                                  derived_from_entry_id IS NULL
                                  OR (
                                      length(derived_from_entry_id) = 36
                                      AND lower(derived_from_entry_id) = derived_from_entry_id
                                      AND derived_from_entry_id GLOB '????????-????-7???-[89ab]???-????????????'
                                      AND replace(derived_from_entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                                  )
                              ),
    revision                  INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    detached_at               INTEGER CHECK (detached_at IS NULL OR detached_at >= 0),
    created_at                INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at                INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (derived_from_entry_id IS NULL OR derived_from_entry_id <> knowledge_entry_id),
    CHECK (
        (relationship = 'managed' AND derived_from_entry_id IS NULL AND detached_at IS NULL)
        OR (relationship = 'detached' AND derived_from_entry_id IS NULL AND detached_at IS NOT NULL)
        OR (relationship = 'copy' AND derived_from_entry_id IS NOT NULL AND detached_at IS NULL)
    )
);

CREATE UNIQUE INDEX uq_knowledge_entry_provenance_entry_id
    ON knowledge_entry_provenance(knowledge_entry_id);

CREATE INDEX idx_knowledge_entry_provenance_source_item_id
    ON knowledge_entry_provenance(knowledge_source_item_id, relationship, knowledge_entry_id);

CREATE INDEX idx_knowledge_entry_provenance_derived_from_entry_id
    ON knowledge_entry_provenance(derived_from_entry_id)
    WHERE derived_from_entry_id IS NOT NULL;

CREATE UNIQUE INDEX uq_knowledge_entry_provenance_managed_source_item
    ON knowledge_entry_provenance(knowledge_source_item_id)
    WHERE relationship = 'managed';
