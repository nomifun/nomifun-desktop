-- Bounded model observations survive Nomi transcript rollback independently.
-- Older settled receipts remain valid but explicitly lack an observation.
ALTER TABLE conversation_mcp_effects ADD COLUMN observation_json TEXT
    CHECK(observation_json IS NULL OR
          (json_valid(observation_json) AND length(CAST(observation_json AS BLOB)) <= 8192));
CREATE INDEX idx_conversation_mcp_history
    ON conversation_mcp_effects(user_id, conversation_id, id DESC);
