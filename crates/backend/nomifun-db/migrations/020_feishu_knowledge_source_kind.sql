-- Widen knowledge_sources.kind so the normalized source aggregate can persist
-- the 'feishu' remote-corpus kind (metadata stubs only; bodies stay remote).
-- SQLite cannot alter a CHECK constraint, so the table is rebuilt under the
-- startup migration's foreign_keys=OFF envelope, then foreign keys are proven
-- clean before commit.
CREATE TABLE knowledge_sources_next (
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
    kind                     TEXT NOT NULL CHECK (kind IN ('url', 'feishu')),
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

INSERT INTO knowledge_sources_next (
    id, knowledge_source_id, knowledge_base_id, kind, mode, state, revision,
    default_parent_entry_id, removed_at, created_at, updated_at
)
SELECT id, knowledge_source_id, knowledge_base_id, kind, mode, state, revision,
       default_parent_entry_id, removed_at, created_at, updated_at
FROM knowledge_sources;

DROP TABLE knowledge_sources;
ALTER TABLE knowledge_sources_next RENAME TO knowledge_sources;

CREATE INDEX idx_knowledge_sources_default_parent_entry_id ON knowledge_sources(default_parent_entry_id) WHERE default_parent_entry_id IS NOT NULL;
CREATE INDEX idx_knowledge_sources_knowledge_base_id ON knowledge_sources(knowledge_base_id, state, created_at, knowledge_source_id);
CREATE UNIQUE INDEX uq_knowledge_sources_live_kind ON knowledge_sources(knowledge_base_id, kind) WHERE state <> 'removed';

CREATE TEMP TABLE feishu_source_integrity (violations INTEGER CHECK (violations = 0));
INSERT INTO feishu_source_integrity SELECT COUNT(*) FROM pragma_foreign_key_check;
DROP TABLE feishu_source_integrity;
PRAGMA legacy_alter_table = OFF;
