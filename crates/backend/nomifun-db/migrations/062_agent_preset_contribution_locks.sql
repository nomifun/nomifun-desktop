-- Persist the generated contribution lock set as part of each immutable
-- AgentPreset revision. The revision digest covers this exact normalized list;
-- dropping it would make a stored revision unverifiable after restart.

ALTER TABLE nomi_agent_preset_revisions
    ADD COLUMN contribution_locks_json TEXT NOT NULL DEFAULT '[]'
    CHECK (json_valid(contribution_locks_json)
           AND json_type(contribution_locks_json) = 'array');
