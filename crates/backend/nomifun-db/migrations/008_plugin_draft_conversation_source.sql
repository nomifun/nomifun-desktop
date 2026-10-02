-- Preserve installed plugins and draft files while linking incremental authoring
-- to the canonical conversation. These are provenance references, not ownership:
-- deleting a conversation must not cascade into a user's plugin or working copy.
ALTER TABLE plugin_drafts ADD COLUMN source_conversation_id TEXT CHECK (
    source_conversation_id IS NULL OR (
        length(source_conversation_id) = 36
        AND lower(source_conversation_id) = source_conversation_id
        AND source_conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(source_conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
);
ALTER TABLE plugin_drafts ADD COLUMN source_message_id TEXT CHECK (
    source_message_id IS NULL OR (
        length(source_message_id) = 36
        AND lower(source_message_id) = source_message_id
        AND source_message_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(source_message_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    )
);
ALTER TABLE plugin_drafts ADD COLUMN source_operation_key TEXT CHECK (
    source_operation_key IS NULL OR length(source_operation_key) BETWEEN 1 AND 512
);
ALTER TABLE plugin_drafts ADD COLUMN source_request_digest TEXT CHECK (
    source_request_digest IS NULL OR (
        length(source_request_digest) = 64
        AND source_request_digest NOT GLOB '*[^0-9a-f]*'
    )
);
ALTER TABLE plugin_drafts ADD COLUMN verification_json TEXT NOT NULL DEFAULT '{}'
    CHECK (json_valid(verification_json) AND json_type(verification_json) = 'object');
CREATE UNIQUE INDEX idx_plugin_drafts_source_operation
    ON plugin_drafts(owner_user_id, source_operation_key)
    WHERE source_operation_key IS NOT NULL;
CREATE INDEX idx_plugin_drafts_source_conversation
    ON plugin_drafts(owner_user_id, source_conversation_id);
