-- Retire the Creator's mutable dialogue and worker state. Preserve the original
-- history as immutable, explicitly imported data, never execution authority.
-- sqlx applies this table replacement in the migration transaction.
CREATE TABLE plugin_drafts_next (
    draft_id TEXT PRIMARY KEY CHECK (
        length(draft_id) = 36 AND lower(draft_id) = draft_id
        AND draft_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(draft_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    owner_user_id TEXT NOT NULL REFERENCES users(user_id) ON DELETE CASCADE CHECK (
        length(owner_user_id) = 36 AND lower(owner_user_id) = owner_user_id
        AND owner_user_id GLOB '????????-????-7???-[89ab]???-????????????'
        AND replace(owner_user_id, '-', '') NOT GLOB '*[^0-9a-f]*'
    ),
    plugin_id TEXT REFERENCES plugins(plugin_id) ON DELETE SET NULL CHECK (
        plugin_id IS NULL OR (length(plugin_id) = 36 AND lower(plugin_id) = plugin_id
            AND plugin_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(plugin_id, '-', '') NOT GLOB '*[^0-9a-f]*')
    ),
    base_revision INTEGER CHECK (base_revision IS NULL OR base_revision > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 160),
    workspace_path TEXT NOT NULL CHECK (length(workspace_path) BETWEEN 1 AND 4096),
    imported_context_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(imported_context_json) AND json_type(imported_context_json) = 'object'),
    status TEXT NOT NULL CHECK (status IN ('ready', 'failed')),
    last_error TEXT,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms > 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms > 0),
    source_conversation_id TEXT CHECK (
        source_conversation_id IS NULL OR (length(source_conversation_id) = 36
            AND lower(source_conversation_id) = source_conversation_id
            AND source_conversation_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(source_conversation_id, '-', '') NOT GLOB '*[^0-9a-f]*')
    ),
    source_message_id TEXT CHECK (
        source_message_id IS NULL OR (length(source_message_id) = 36
            AND lower(source_message_id) = source_message_id
            AND source_message_id GLOB '????????-????-7???-[89ab]???-????????????'
            AND replace(source_message_id, '-', '') NOT GLOB '*[^0-9a-f]*')
    ),
    source_operation_key TEXT CHECK (source_operation_key IS NULL OR length(source_operation_key) BETWEEN 1 AND 512),
    source_request_digest TEXT CHECK (source_request_digest IS NULL OR (
        length(source_request_digest) = 64 AND source_request_digest NOT GLOB '*[^0-9a-f]*'
    )),
    verification_json TEXT NOT NULL DEFAULT '{}'
        CHECK (json_valid(verification_json) AND json_type(verification_json) = 'object'),
    CHECK ((plugin_id IS NULL AND base_revision IS NULL) OR plugin_id IS NOT NULL)
);

INSERT INTO plugin_drafts_next (
    draft_id, owner_user_id, plugin_id, base_revision, revision, name, workspace_path,
    imported_context_json, status, last_error, created_at_ms, updated_at_ms,
    source_conversation_id, source_message_id, source_operation_key, source_request_digest, verification_json
)
SELECT draft_id, owner_user_id, plugin_id, base_revision,
    revision + CASE WHEN status = 'generating' THEN 1 ELSE 0 END, name, workspace_path,
    CASE WHEN json_array_length(messages_json) > 0 OR status = 'generating' THEN
        json_object('source', 'legacy_plugin_creator', 'data_only', json('true'),
            'messages', json(messages_json), 'legacy_status', status)
        ELSE '{}' END,
    CASE WHEN status = 'generating' THEN 'failed' ELSE status END,
    CASE WHEN status = 'generating' THEN 'PLUGIN_GENERATION_INTERRUPTED' ELSE last_error END,
    created_at_ms, updated_at_ms, source_conversation_id, source_message_id,
    source_operation_key, source_request_digest, verification_json
FROM plugin_drafts;

DROP TABLE plugin_drafts;
ALTER TABLE plugin_drafts_next RENAME TO plugin_drafts;
CREATE INDEX idx_plugin_drafts_owner_updated ON plugin_drafts(owner_user_id, updated_at_ms DESC, draft_id);
CREATE UNIQUE INDEX idx_plugin_drafts_source_operation
    ON plugin_drafts(owner_user_id, source_operation_key) WHERE source_operation_key IS NOT NULL;
CREATE INDEX idx_plugin_drafts_source_conversation ON plugin_drafts(owner_user_id, source_conversation_id);

CREATE TRIGGER trg_plugin_drafts_identity_immutable
BEFORE UPDATE ON plugin_drafts
WHEN NEW.draft_id IS NOT OLD.draft_id OR NEW.owner_user_id IS NOT OLD.owner_user_id
    OR NEW.created_at_ms IS NOT OLD.created_at_ms OR NEW.updated_at_ms < OLD.updated_at_ms
    OR NEW.imported_context_json IS NOT OLD.imported_context_json
BEGIN
    SELECT RAISE(ABORT, 'Plugin Draft identity, imported history and time are immutable');
END;

-- A saved draft remains a historical source after permanent Plugin deletion.
-- Clear the base before ON DELETE SET NULL, preserving the identity constraint.
CREATE TRIGGER trg_plugin_drafts_clear_base_before_plugin_delete
BEFORE DELETE ON plugins
BEGIN
    UPDATE plugin_drafts SET base_revision = NULL, revision = revision + 1 WHERE plugin_id = OLD.plugin_id;
END;
