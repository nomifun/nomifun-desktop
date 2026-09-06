-- Product deletion for AgentPreset is a retirement tombstone.
--
-- Immutable revisions, snapshots, and historical sessions remain addressable
-- by their frozen identities. Active product surfaces filter this column.

ALTER TABLE nomi_agent_presets
    ADD COLUMN retired_at_ms INTEGER
    CHECK (retired_at_ms IS NULL OR retired_at_ms >= 0);

CREATE INDEX idx_nomi_agent_presets_active_owner
    ON nomi_agent_presets(owner_user_id, preset_id)
    WHERE retired_at_ms IS NULL;
