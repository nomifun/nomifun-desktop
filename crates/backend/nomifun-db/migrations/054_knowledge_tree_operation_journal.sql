-- Durable coordinator journal and transactional outbox for knowledge-tree
-- mutations. File contents and directory structure remain filesystem-owned;
-- this row records enough intent and progress to make a rename recoverable and
-- an event publish retryable after a process crash.
--
-- Relationships follow the v3 logical-reference convention: canonical TEXT
-- IDs plus indexes, with repository-coordinated cleanup and no physical keys.

CREATE TABLE knowledge_tree_operations (
    id                       INTEGER PRIMARY KEY AUTOINCREMENT,
    operation_id             TEXT NOT NULL UNIQUE
                             CHECK (
                                 length(operation_id) = 36
                                 AND lower(operation_id) = operation_id
                                 AND operation_id GLOB '????????-????-7???-[89ab]???-????????????'
                                 AND replace(operation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                             ),
    knowledge_base_id        TEXT NOT NULL
                             CHECK (
                                 length(knowledge_base_id) = 36
                                 AND lower(knowledge_base_id) = knowledge_base_id
                                 AND knowledge_base_id GLOB '????????-????-7???-[89ab]???-????????????'
                                 AND replace(knowledge_base_id, '-', '') NOT GLOB '*[^0-9a-f]*'
                             ),
    request_id               TEXT NOT NULL
                             CHECK (
                                 length(request_id) BETWEEN 1 AND 128
                                 AND request_id NOT GLOB '*[^!-~]*'
                             ),
    fingerprint              TEXT NOT NULL
                             CHECK (
                                 length(fingerprint) = 64
                                 AND lower(fingerprint) = fingerprint
                                 AND fingerprint NOT GLOB '*[^0-9a-f]*'
                             ),
    source_rel_path          TEXT NOT NULL
                             CHECK (
                                 source_rel_path <> ''
                                 AND substr(source_rel_path, 1, 1) <> '/'
                                 AND substr(source_rel_path, -1, 1) <> '/'
                                 AND instr(source_rel_path, '\') = 0
                                 AND instr(source_rel_path, '//') = 0
                                 AND instr(source_rel_path, char(0)) = 0
                             ),
    destination_rel_path     TEXT NOT NULL
                             CHECK (
                                 destination_rel_path <> ''
                                 AND substr(destination_rel_path, 1, 1) <> '/'
                                 AND substr(destination_rel_path, -1, 1) <> '/'
                                 AND instr(destination_rel_path, '\') = 0
                                 AND instr(destination_rel_path, '//') = 0
                                 AND instr(destination_rel_path, char(0)) = 0
                             ),
    -- Physical identity observed while intent is prepared. On platforms that
    -- expose inode/file-index identity, recovery uses this to distinguish our
    -- completed rename from an unrelated file later occupying the target.
    source_fs_identity       TEXT CHECK (
                                 source_fs_identity IS NULL
                                 OR length(source_fs_identity) BETWEEN 1 AND 512
                             ),
    state                    TEXT NOT NULL DEFAULT 'prepared'
                             CHECK (state IN (
                                 'prepared',
                                 'filesystem_committed',
                                 'committed',
                                 'needs_recovery'
                             )),
    receipt_json             TEXT
                             CHECK (receipt_json IS NULL OR json_valid(receipt_json)),
    error_message            TEXT
                             CHECK (
                                 error_message IS NULL
                                 OR length(error_message) BETWEEN 1 AND 8192
                             ),
    event_status             TEXT NOT NULL DEFAULT 'none'
                             CHECK (event_status IN ('none', 'pending', 'published')),
    event_payload_json       TEXT
                             CHECK (
                                 event_payload_json IS NULL
                                 OR json_valid(event_payload_json)
                             ),
    filesystem_committed_at  INTEGER,
    committed_at             INTEGER,
    event_published_at       INTEGER,
    created_at               INTEGER NOT NULL CHECK (created_at >= 0),
    updated_at               INTEGER NOT NULL CHECK (updated_at >= created_at),

    UNIQUE (knowledge_base_id, request_id),

    CHECK (
        filesystem_committed_at IS NULL
        OR filesystem_committed_at >= created_at
    ),
    CHECK (
        committed_at IS NULL
        OR (
            filesystem_committed_at IS NOT NULL
            AND committed_at >= filesystem_committed_at
        )
    ),
    CHECK (
        event_published_at IS NULL
        OR (
            committed_at IS NOT NULL
            AND event_published_at >= committed_at
        )
    ),
    CHECK (
        (state = 'prepared'
            AND filesystem_committed_at IS NULL
            AND committed_at IS NULL
            AND receipt_json IS NULL
            AND error_message IS NULL)
        OR
        (state = 'filesystem_committed'
            AND filesystem_committed_at IS NOT NULL
            AND committed_at IS NULL
            AND receipt_json IS NULL
            AND error_message IS NULL)
        OR
        (state = 'committed'
            AND filesystem_committed_at IS NOT NULL
            AND committed_at IS NOT NULL
            AND receipt_json IS NOT NULL
            AND error_message IS NULL)
        OR
        (state = 'needs_recovery'
            AND committed_at IS NULL
            AND receipt_json IS NULL
            AND error_message IS NOT NULL)
    ),
    CHECK (
        (state = 'committed'
            AND event_status IN ('pending', 'published')
            AND event_payload_json IS NOT NULL)
        OR
        (state <> 'committed'
            AND event_status = 'none'
            AND event_payload_json IS NULL
            AND event_published_at IS NULL)
    ),
    CHECK (
        (event_status = 'published' AND event_published_at IS NOT NULL)
        OR
        (event_status <> 'published' AND event_published_at IS NULL)
    )
);

CREATE INDEX idx_knowledge_tree_operations_knowledge_base_id
    ON knowledge_tree_operations(knowledge_base_id, created_at, operation_id);

CREATE INDEX idx_knowledge_tree_operations_recovery
    ON knowledge_tree_operations(state, created_at, operation_id)
    WHERE state <> 'committed';

CREATE INDEX idx_knowledge_tree_operations_pending_events
    ON knowledge_tree_operations(event_status, committed_at, operation_id)
    WHERE event_status = 'pending';
