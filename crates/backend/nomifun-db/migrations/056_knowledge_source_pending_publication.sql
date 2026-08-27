-- Durable two-phase publication intent for managed source documents.
--
-- A refresher records the exact prepared hash and metadata before replacing the
-- filesystem document. If the process stops between the filesystem effect and
-- sync settlement, startup can compare the live file with this row and either
-- finish the success transition or preserve a conflicting local edit.

ALTER TABLE knowledge_source_items
    ADD COLUMN pending_published_hash TEXT
    CHECK (
        pending_published_hash IS NULL
        OR (
            length(pending_published_hash) = 64
            AND lower(pending_published_hash) = pending_published_hash
            AND pending_published_hash NOT GLOB '*[^0-9a-f]*'
            AND state = 'active'
            AND sync_status = 'syncing'
        )
    );

ALTER TABLE knowledge_source_items
    ADD COLUMN pending_final_url TEXT
    CHECK (
        pending_final_url IS NULL
        OR (
            pending_published_hash IS NOT NULL
            AND length(pending_final_url) BETWEEN 1 AND 8192
            AND trim(pending_final_url) = pending_final_url
            AND instr(pending_final_url, char(0)) = 0
        )
    );

ALTER TABLE knowledge_source_items
    ADD COLUMN pending_title TEXT
    CHECK (
        pending_title IS NULL
        OR (
            pending_published_hash IS NOT NULL
            AND length(pending_title) BETWEEN 1 AND 1024
            AND trim(pending_title) = pending_title
            AND instr(pending_title, char(0)) = 0
        )
    );

ALTER TABLE knowledge_source_items
    ADD COLUMN pending_publication_at INTEGER
    CHECK (
        (pending_published_hash IS NULL AND pending_publication_at IS NULL)
        OR (
            pending_published_hash IS NOT NULL
            AND pending_publication_at IS NOT NULL
            AND pending_publication_at >= 0
        )
    );
