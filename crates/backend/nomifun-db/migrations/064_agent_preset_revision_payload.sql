-- Canonicalize the immutable AgentPreset revision payload column name.
--
-- The stored value is the canonical AgentPresetRevision payload, not an
-- editor-owned document. This is a physical rename only: no compatibility
-- view, alias, or dual-column read/write path is introduced.

ALTER TABLE nomi_agent_preset_revisions
    RENAME COLUMN editor_document_json TO payload_json;
