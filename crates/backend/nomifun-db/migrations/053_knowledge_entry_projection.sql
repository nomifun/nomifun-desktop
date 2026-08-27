-- Stable identities for filesystem-backed knowledge entries.
--
-- Markdown and directory contents remain owned by the filesystem. This table
-- is a rebuildable identity/location projection: `knowledge_entry_id`
-- survives a rename or move, while `rel_path` and `portable_rel_path` are
-- derived location caches used for reconciliation and collision checks.
-- Relationships intentionally follow the v3 logical-reference convention:
-- indexed UUIDv7 TEXT columns, with repository-coordinated cleanup and no
-- physical SQLite foreign keys.

ALTER TABLE knowledge_bases
    ADD COLUMN tree_revision INTEGER NOT NULL DEFAULT 0
    CHECK (tree_revision >= 0);

CREATE TABLE knowledge_entries (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    knowledge_entry_id  TEXT NOT NULL UNIQUE
                        CHECK (
                            length(knowledge_entry_id) = 36
                            AND lower(knowledge_entry_id) = knowledge_entry_id
                            AND knowledge_entry_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(knowledge_entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    knowledge_base_id   TEXT NOT NULL
                        CHECK (
                            length(knowledge_base_id) = 36
                            AND lower(knowledge_base_id) = knowledge_base_id
                            AND knowledge_base_id GLOB '????????-????-7???-[89ab]???-????????????'
                            AND replace(knowledge_base_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                        ),
    parent_entry_id     TEXT
                        CHECK (
                            parent_entry_id IS NULL
                            OR (
                                length(parent_entry_id) = 36
                                AND lower(parent_entry_id) = parent_entry_id
                                AND parent_entry_id GLOB '????????-????-7???-[89ab]???-????????????'
                                AND replace(parent_entry_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                            )
                        ),
    name                TEXT NOT NULL
                        CHECK (
                            name <> ''
                            AND name NOT IN ('.', '..')
                            AND instr(name, '/') = 0
                            AND instr(name, '\') = 0
                            AND instr(name, char(0)) = 0
                        ),
    kind                TEXT NOT NULL CHECK (kind IN ('file', 'directory')),
    origin              TEXT NOT NULL CHECK (origin IN ('user', 'url_snapshot', 'generated')),
    rel_path            TEXT NOT NULL
                        CHECK (
                            rel_path <> ''
                            AND substr(rel_path, 1, 1) <> '/'
                            AND substr(rel_path, -1, 1) <> '/'
                            AND instr(rel_path, '\') = 0
                            AND instr(rel_path, '//') = 0
                            AND instr(rel_path, char(0)) = 0
                        ),
    portable_rel_path   TEXT NOT NULL
                        CHECK (
                            portable_rel_path <> ''
                            AND substr(portable_rel_path, 1, 1) <> '/'
                            AND substr(portable_rel_path, -1, 1) <> '/'
                            AND instr(portable_rel_path, '\') = 0
                            AND instr(portable_rel_path, '//') = 0
                            AND instr(portable_rel_path, char(0)) = 0
                        ),
    fs_identity         TEXT,
    content_hash        TEXT,
    revision            INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    deleted_at          INTEGER,
    created_at          INTEGER NOT NULL,
    updated_at          INTEGER NOT NULL,
    CHECK (parent_entry_id IS NULL OR parent_entry_id <> knowledge_entry_id),
    CHECK (deleted_at IS NULL OR deleted_at >= created_at),
    CHECK (updated_at >= created_at)
);

CREATE INDEX idx_knowledge_entries_knowledge_base_id
    ON knowledge_entries(knowledge_base_id, parent_entry_id, deleted_at, name);

CREATE INDEX idx_knowledge_entries_parent_entry_id
    ON knowledge_entries(parent_entry_id, deleted_at);

CREATE UNIQUE INDEX uq_knowledge_entries_live_rel_path
    ON knowledge_entries(knowledge_base_id, rel_path)
    WHERE deleted_at IS NULL;

CREATE UNIQUE INDEX uq_knowledge_entries_live_portable_path
    ON knowledge_entries(knowledge_base_id, portable_rel_path)
    WHERE deleted_at IS NULL;

CREATE INDEX idx_knowledge_entries_fs_identity
    ON knowledge_entries(knowledge_base_id, fs_identity)
    WHERE fs_identity IS NOT NULL AND deleted_at IS NULL;

CREATE INDEX idx_knowledge_entries_content_hash
    ON knowledge_entries(knowledge_base_id, content_hash)
    WHERE content_hash IS NOT NULL AND deleted_at IS NULL;
